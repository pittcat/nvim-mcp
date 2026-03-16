use std::{path::PathBuf, str::FromStr, sync::OnceLock, time::Instant};

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
use tracing_subscriber::EnvFilter;

use nvim_mcp::{
    NeovimMcpServer, auto_connect_current_project_targets, auto_connect_single_target,
    logging::{default_debug_log_path, init_debug_logging},
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

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use nvim_mcp::logging::prepare_debug_log_file;

    #[test]
    fn prepare_debug_log_file_recreates_empty_file() {
        let dir = tempdir().expect("tempdir");
        let log_path = dir.path().join("debug_log.txt");

        fs::write(&log_path, "old log content").expect("seed log");
        prepare_debug_log_file(&log_path).expect("prepare log");

        let content = fs::read_to_string(&log_path).expect("read recreated log");
        assert!(content.is_empty(), "log file should be recreated empty");
    }
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

fn normalize_file_log_level(level: &str) -> &'static str {
    match level.to_ascii_lowercase().as_str() {
        "trace" => "trace",
        "debug" => "debug",
        "info" | "warn" | "error" => "debug",
        _ => "debug",
    }
}

fn preview_http_header(request: &Request<Incoming>, name: &str) -> String {
    request
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| nvim_mcp::logging::preview_text(value, 120))
        .unwrap_or_else(|| "-".to_string())
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
    let start_time = Instant::now();
    let cli = Cli::parse();

    let file_log_level = normalize_file_log_level(&cli.log_level);
    let env_filter = EnvFilter::from_default_env().add_directive(file_log_level.parse()?);
    let debug_log_path = default_debug_log_path()?;
    let _guard = init_debug_logging(&debug_log_path, env_filter)?;

    info!(
        phase = "START",
        context_id = "bootstrap",
        "========== 任务开始 =========="
    );
    info!(
        context_id = "bootstrap",
        "环境: {} | 版本: {} | 依赖: rmcp=0.14, nvim-rs=0.9.2, tracing=0.1.44 | 日志文件: {}",
        std::env::var("NVIM_MCP_ENV")
            .or_else(|_| std::env::var("APP_ENV"))
            .unwrap_or_else(|_| "dev".to_string()),
        long_version(),
        debug_log_path.display()
    );
    info!(
        context_id = "bootstrap",
        "配置: connect={} | http_host={} | http_port={} | cli_log_level={} | file_log_level={}",
        cli.connect,
        cli.http_host,
        cli.http_port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "stdio".to_string()),
        cli.log_level,
        file_log_level
    );
    info!(
        context_id = "bootstrap",
        "运行信息: pid={} | cwd={} | args={:?}",
        std::process::id(),
        std::env::current_dir()?.display(),
        std::env::args().collect::<Vec<_>>()
    );
    if let Some(log_file) = &cli.log_file {
        warn!(
            context_id = "bootstrap",
            "忽略 --log-file={}，当前调试模式固定输出到 {}",
            log_file.display(),
            debug_log_path.display()
        );
    }

    let result = run_server(cli).await;
    match &result {
        Ok(_) => info!(
            phase = "END",
            context_id = "bootstrap",
            "========== 任务完成 | 总耗时: {:.3}s ==========",
            start_time.elapsed().as_secs_f64()
        ),
        Err(error) => error!(
            phase = "END",
            context_id = "bootstrap",
            "========== 任务失败 | 总耗时: {:.3}s | 错误: {} ==========",
            start_time.elapsed().as_secs_f64(),
            error
        ),
    }

    result
}

async fn run_server(cli: Cli) -> Result<(), Box<dyn std::error::Error>> {
    info!(context_id = "bootstrap", "Starting nvim-mcp Neovim server");
    let connect_mode = cli.connect.to_string();
    let server = NeovimMcpServer::with_connect_mode(Some(connect_mode.clone()));
    info!(
        context_id = "bootstrap",
        "初始化服务实例 | 调用栈: main() → run_server() line {} | 数据流: connect_mode={}",
        line!(),
        connect_mode
    );

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
        info!(
            context_id = "bootstrap",
            "进入 HTTP 模式 | 调用栈: run_server() line {} | 数据流: bind_addr={} stateful_mode=true channel_capacity=64",
            line!(),
            addr
        );

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
                    info!(
                        context_id = "http_accept",
                        "接受 HTTP 连接 | 调用栈: run_server() → listener.accept() line {} | 数据流: peer_addr={}",
                        line!(),
                        peer_addr
                    );
                    (TokioIo::new(stream), peer_addr)
                }
            };
            let service = service.clone();
            tokio::spawn(async move {
                info!(
                    context_id = "http_connection",
                    "开始处理 HTTP 连接 | 调用栈: run_server() → serve_connection() line {}",
                    line!()
                );
                let request_logging_service = service_fn(move |request: Request<Incoming>| {
                    let service = service.clone();
                    let peer_addr = peer_addr;
                    let method = request.method().clone();
                    let uri = request.uri().clone();
                    let user_agent = preview_http_header(&request, "user-agent");
                    let content_type = preview_http_header(&request, "content-type");
                    let accept = preview_http_header(&request, "accept");
                    let session_id = preview_http_header(&request, "mcp-session-id");
                    let context_id = format!("http:{}:{}:{}", peer_addr, method, uri.path());

                    info!(
                        context_id = %context_id,
                        "收到 HTTP 请求 | 调用栈: run_server() → service_fn() line {} | 数据流: peer_addr={} method={} uri={} user_agent={} content_type={} accept={} session_id={}",
                        line!(),
                        peer_addr,
                        method,
                        uri,
                        user_agent,
                        content_type,
                        accept,
                        session_id
                    );

                    async move {
                        let response = Service::call(&service, request).await;
                        match &response {
                            Ok(response) => info!(
                                context_id = %context_id,
                                "HTTP 请求处理完成 | 调用栈: service_fn() line {} | 数据流: status={}",
                                line!(),
                                response.status()
                            ),
                            Err(error) => warn!(
                                context_id = %context_id,
                                "HTTP 请求处理失败 | 调用栈: service_fn() line {} | 数据流: error={}",
                                line!(),
                                error
                            ),
                        }
                        response
                    }
                });

                if let Err(e) = Builder::new(TokioExecutor::default())
                    .serve_connection(io, request_logging_service)
                    .await
                {
                    error!(
                        context_id = "http_connection",
                        "处理 HTTP 连接失败 | 调用栈: serve_connection() line {} | 数据流: error={}",
                        line!(),
                        e
                    );
                } else {
                    info!(
                        context_id = "http_connection",
                        "HTTP 连接处理完成 | 调用栈: serve_connection() line {}",
                        line!()
                    );
                }
            });
        }
    } else {
        // Default stdio mode
        info!("Starting Neovim server on stdio");
        info!(
            context_id = "bootstrap",
            "进入 stdio 模式 | 调用栈: run_server() line {} | 数据流: transport=stdio",
            line!()
        );
        let service = server.serve(stdio()).await.inspect_err(|e| {
            error!("Error starting Neovim server: {}", e);
        })?;

        info!("Neovim server started, waiting for connections...");
        service.waiting().await?;
    };
    info!(context_id = "bootstrap", "Server shutdown complete");

    Ok(())
}
