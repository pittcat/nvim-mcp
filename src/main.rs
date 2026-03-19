use std::{path::PathBuf, str::FromStr, sync::OnceLock};

use clap::Parser;
use hyper::{
    Request,
    body::Incoming,
    service::{Service, service_fn},
};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use rmcp::{
    ServiceExt,
    transport::{
        StreamableHttpServerConfig, StreamableHttpService, stdio,
        streamable_http_server::session::local::{LocalSessionManager, SessionConfig},
    },
};
use tracing::{error, info, warn};

use nvim_mcp::{
    NeovimMcpServer, auto_connect_current_project_targets, auto_connect_single_target,
    logging::init_logging,
};

static LONG_VERSION: OnceLock<String> = OnceLock::new();

fn long_version() -> &'static str {
    LONG_VERSION
        .get_or_init(|| {
            // This closure is executed only once, on the first call to get_or_init
            let dirty = if env!("GIT_DIRTY") == "true" {
                "[dirty]"
            } else {
                ""
            };
            format!(
                "{} (sha:{:?}, build_time:{:?}){}",
                env!("CARGO_PKG_VERSION"),
                env!("GIT_COMMIT_SHA"),
                env!("BUILT_TIME_UTC"),
                dirty
            )
        })
        .as_str()
}


#[derive(Clone, Debug)]
enum ConnectBehavior {
    Manual,
    Auto,
    SpecificTarget(String),
}

impl std::fmt::Display for ConnectBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectBehavior::Manual => write!(f, "manual"),
            ConnectBehavior::Auto => write!(f, "auto"),
            ConnectBehavior::SpecificTarget(target) => write!(f, "{}", target),
        }
    }
}

impl FromStr for ConnectBehavior {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "manual" => Ok(ConnectBehavior::Manual),
            "auto" => Ok(ConnectBehavior::Auto),
            target => {
                // Validate TCP address format
                if target.parse::<std::net::SocketAddr>().is_ok() {
                    return Ok(ConnectBehavior::SpecificTarget(target.to_string()));
                }

                // Validate file path (socket/pipe)
                let path = std::path::Path::new(target);
                if path.is_absolute()
                    && (path.exists() || path.parent().is_some_and(|p| p.exists()))
                {
                    return Ok(ConnectBehavior::SpecificTarget(target.to_string()));
                }

                Err(format!(
                    "Invalid target: '{}'. Must be 'manual', 'auto', TCP address (e.g., '127.0.0.1:6666'), or absolute socket path",
                    target
                ))
            }
        }
    }
}

#[derive(Parser)]
#[command(version, long_version=long_version(), about, long_about = None)]
struct Cli {
    /// Path to the log file. If not specified, logs to stderr
    #[arg(long)]
    log_file: Option<PathBuf>,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "debug")]
    log_level: String,

    /// Enable HTTP server mode on the specified port
    #[arg(long)]
    http_port: Option<u16>,

    /// HTTP server bind address (default: 127.0.0.1)
    #[arg(long, default_value = "127.0.0.1")]
    http_host: String,

    /// Connection mode: 'manual', 'auto', or specific target (TCP address/socket path)
    #[arg(long, default_value = "manual")]
    connect: ConnectBehavior,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Initialize logging only for HTTP server mode (not stdio mode)
    // In stdio mode, logs would interfere with the MCP protocol on stdin/stdout
    let _guard = if cli.http_port.is_some() {
        init_logging(cli.log_file.as_ref(), &cli.log_level)?
    } else {
        None
    };

    info!("Starting nvim-mcp v{}", long_version());

    let result = run_server(cli).await;
    if let Err(error) = &result {
        error!("Server error: {}", error);
    }

    result
}

async fn run_server(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    let connect_mode = cli.connect.to_string();
    let server = NeovimMcpServer::with_connect_mode(Some(connect_mode.clone()));

    // Handle connection mode
    let connection_ids = match cli.connect {
        ConnectBehavior::Auto => {
            match auto_connect_current_project_targets(&server).await {
                Ok(connections) => {
                    if connections.is_empty() {
                        info!("No Neovim instances found for current project");
                    } else {
                        info!("Auto-connected to {} project instances", connections.len());
                    }
                    connections
                }
                Err(failures) => {
                    warn!("Auto-connection failed for all {} targets", failures.len());
                    for (target, error) in &failures {
                        warn!("  {target}: {error}");
                    }
                    // Continue serving - manual connections still possible
                    vec![]
                }
            }
        }
        ConnectBehavior::SpecificTarget(target) => {
            match auto_connect_single_target(&server, &target).await {
                Ok(id) => {
                    info!("Connected to specific target {} with ID {}", target, id);
                    vec![id]
                }
                Err(e) => return Err(format!("Failed to connect to {}: {}", target, e).into()),
            }
        }
        ConnectBehavior::Manual => {
            info!("Manual connection mode - use get_targets and connect tools");
            vec![]
        }
    };

    if !connection_ids.is_empty() {
        server
            .discover_and_register_lua_tools()
            .await
            .inspect_err(|e| {
                error!("Error setting up Lua tools: {}", e);
            })?;
    }

    if let Some(port) = cli.http_port {
        // HTTP server mode
        let addr = format!("{}:{}", cli.http_host, port);
        info!("Starting HTTP server on {}", addr);

        // Configure session manager with appropriate settings for multi-client scenarios:
        // - Increased channel capacity to handle concurrent requests from multiple clients
        // - No keep-alive timeout to prevent premature session closure
        let session_config = SessionConfig {
            channel_capacity: 64, // Increased from default 16 for multi-client support
            keep_alive: None,     // No timeout - sessions stay alive until explicitly closed
        };
        let session_manager = LocalSessionManager {
            sessions: tokio::sync::RwLock::new(std::collections::HashMap::new()),
            session_config,
        };

        let service = TowerToHyperService::new(StreamableHttpService::new(
            move || Ok(server.server_for_http_session()),
            session_manager.into(),
            StreamableHttpServerConfig {
                stateful_mode: true,
                ..Default::default()
            },
        ));
        let listener = tokio::net::TcpListener::bind(addr).await?;
        loop {
            let (io, peer_addr) = tokio::select! {
                _ = tokio::signal::ctrl_c() => break,
                accept = listener.accept() => {
                    let (stream, peer_addr) = accept?;
                    info!("Accepted connection from {}", peer_addr);
                    (TokioIo::new(stream), peer_addr)
                }
            };
            let service = service.clone();
            tokio::spawn(async move {
                let request_logging_service = service_fn(move |request: Request<Incoming>| {
                    let service = service.clone();
                    let peer_addr = peer_addr;
                    let method = request.method().clone();
                    let uri = request.uri().clone();
                    let context_id = format!("http:{}:{}:{}", peer_addr, method, uri.path());

                    info!(context_id = %context_id, "HTTP request: {} {} from {}", method, uri, peer_addr);

                    async move {
                        let response = Service::call(&service, request).await;
                        match &response {
                            Ok(response) => info!(context_id = %context_id, "HTTP response: {}", response.status()),
                            Err(error) => warn!(context_id = %context_id, "HTTP error: {}", error),
                        }
                        response
                    }
                });

                if let Err(e) = Builder::new(TokioExecutor::default())
                    .serve_connection(io, request_logging_service)
                    .await
                {
                    error!("HTTP connection error: {}", e);
                }
            });
        }
    } else {
        // Default stdio mode
        info!("Starting Neovim server on stdio");
        let service = server.serve(stdio()).await.inspect_err(|e| {
            error!("Error starting Neovim server: {}", e);
        })?;

        info!("Neovim server started, waiting for connections...");
        service.waiting().await?;
    };

    Ok(())
}
