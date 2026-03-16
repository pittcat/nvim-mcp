use std::process::Command;
use std::sync::Arc;

use dashmap::DashMap;
use rmcp::{ErrorData as McpError, RoleServer, service::RequestContext};
use tracing::{debug, info, warn};

use crate::{
    logging::{preview_text, request_context_id},
    neovim::{NeovimClientTrait, NeovimError},
    server::{
        hybrid_router::{DynamicToolBox, HybridToolRouter},
        lua_tools,
    },
};

impl From<NeovimError> for McpError {
    fn from(err: NeovimError) -> Self {
        match err {
            NeovimError::Connection(msg) => McpError::invalid_request(msg, None),
            NeovimError::Api(msg) => McpError::internal_error(msg, None),
        }
    }
}

#[derive(Clone)]
pub struct NeovimMcpServer {
    pub nvim_clients: Arc<DashMap<String, Box<dyn NeovimClientTrait + Send>>>,
    pub hybrid_router: HybridToolRouter,
    pub connect_mode: Option<String>,
}

impl NeovimMcpServer {
    pub fn new() -> Self {
        Self::with_connect_mode(None)
    }

    pub fn with_connect_mode(connect_mode: Option<String>) -> Self {
        debug!("Creating new NeovimMcpServer instance");
        let static_router = crate::server::tools::build_tool_router();
        let static_tool_descriptions = Self::tool_descriptions();
        Self {
            nvim_clients: Arc::new(DashMap::new()),
            hybrid_router: HybridToolRouter::new(static_router, static_tool_descriptions),
            connect_mode,
        }
    }

    pub fn router(&self) -> &HybridToolRouter {
        &self.hybrid_router
    }

    /// Generate shorter connection ID with collision detection
    pub fn generate_shorter_connection_id(&self, target: &str) -> String {
        let full_hash = b3sum(target);
        let id_length = 7;

        // Try different starting positions in the hash for 7-char IDs
        for start in 0..=(full_hash.len().saturating_sub(id_length)) {
            let candidate = &full_hash[start..start + id_length];

            if let Some(existing_client) = self.nvim_clients.get(candidate) {
                // Check if the existing connection has the same target
                if let Some(existing_target) = existing_client.target()
                    && existing_target == target
                {
                    // Same target, return existing connection ID (connection replacement)
                    return candidate.to_string();
                }
                // Different target, continue looking for another ID
                continue;
            }

            // No existing connection with this ID, safe to use
            return candidate.to_string();
        }

        // Fallback to full hash if somehow all combinations are taken
        full_hash
    }

    /// Get connection by ID with proper error handling
    pub fn get_connection(
        &'_ self,
        connection_id: &str,
    ) -> Result<dashmap::mapref::one::Ref<'_, String, Box<dyn NeovimClientTrait + Send>>, McpError>
    {
        match self.nvim_clients.get(connection_id) {
            Some(connection) => {
                debug!(
                    context_id = connection_id,
                    "连接查询成功 | 调用栈: server() → get_connection() line {} | 数据流: 输入 connection_id={} → 输出 target={}",
                    line!(),
                    connection_id,
                    connection
                        .target()
                        .map(|target| preview_text(&target, 80))
                        .unwrap_or_else(|| "Unknown".to_string())
                );
                Ok(connection)
            }
            None => {
                warn!(
                    context_id = connection_id,
                    "连接查询失败 | 调用栈: server() → get_connection() line {} | 数据流: 输入 connection_id={} → 输出 not_found",
                    line!(),
                    connection_id
                );
                Err(McpError::invalid_request(
                    format!("No Neovim connection found for ID: {connection_id}"),
                    None,
                ))
            }
        }
    }

    /// Get dynamic connections info for LLM
    pub fn get_connections_instruction(&self) -> String {
        let mut instructions = String::from("## Connection Status\n\n");

        // Add connection status section

        if let Some(ref connect_mode) = self.connect_mode {
            instructions.push_str(&format!("Connection mode: `{}`\n\n", connect_mode));
        }

        // Show active connections with their IDs
        let connections: Vec<_> = self
            .nvim_clients
            .iter()
            .map(|entry| {
                let connection_id = entry.key();
                let target = entry
                    .value()
                    .target()
                    .unwrap_or_else(|| "Unknown".to_string());
                format!(
                    "- **Connection ID: `{}`** → Target: `{}`",
                    connection_id, target
                )
            })
            .collect();

        if connections.is_empty() {
            instructions.push_str("**Active Connections:** None\n\n");
        } else {
            instructions.push_str("**Active Connections:**\n\n");
            for connection in connections {
                instructions.push_str(&format!("{}\n", connection));
            }
            instructions.push_str("\n**Ready to use!** You can immediately use any connection-aware tools with the connection IDs above.");
        }

        instructions
    }

    /// Register a connection-specific tool with clean name
    pub fn register_dynamic_tool(
        &self,
        connection_id: &str,
        tool: DynamicToolBox,
    ) -> Result<(), McpError> {
        self.hybrid_router
            .register_dynamic_tool(connection_id, tool)
    }

    /// Remove all dynamic tools for a connection
    pub fn unregister_dynamic_tools(&self, connection_id: &str) {
        self.hybrid_router.unregister_dynamic_tools(connection_id)
    }

    /// Get count of dynamic tools for a connection
    pub fn get_dynamic_tool_count(&self, connection_id: &str) -> usize {
        self.hybrid_router.get_connection_tool_count(connection_id)
    }

    pub async fn discover_and_register_lua_tools(&self) -> Result<(), McpError> {
        for item in self.nvim_clients.iter() {
            let connection_id = item.key().as_str();
            let client = item.value().as_ref();
            info!(
                context_id = connection_id,
                "开始发现 Lua tools | 调用栈: server() → discover_and_register_lua_tools() line {} | 数据流: 输入 connection_id={} target={}",
                line!(),
                connection_id,
                client
                    .target()
                    .map(|target| preview_text(&target, 80))
                    .unwrap_or_else(|| "Unknown".to_string())
            );
            lua_tools::discover_and_register_lua_tools(self, connection_id, client).await?;
        }
        Ok(())
    }

    pub fn server_for_http_session(&self) -> Self {
        self.clone()
    }

    pub(crate) async fn setup_new_client(
        &self,
        connection_id: &String,
        client: Box<dyn NeovimClientTrait + Send + Sync>,
        ctx: &RequestContext<RoleServer>,
    ) -> Result<(), McpError> {
        let context_id = request_context_id(ctx, "setup_client");
        let mut should_notify = self.nvim_clients.is_empty();
        info!(
            context_id = %context_id,
            "注册新客户端 | 调用栈: setup_new_client() line {} | 数据流: connection_id={} → nvim_clients_before={}",
            line!(),
            connection_id,
            self.nvim_clients.len()
        );

        // Discover and register Lua tools for this connection
        if let Err(e) =
            lua_tools::discover_and_register_lua_tools(self, connection_id, client.as_ref()).await
        {
            tracing::warn!(
                context_id = %context_id,
                "Failed to discover Lua tools for connection '{}': {}",
                connection_id,
                e
            );
        } else {
            should_notify = true;
        }

        self.nvim_clients.insert(connection_id.clone(), client);

        if should_notify {
            ctx.peer
                .notify_tool_list_changed()
                .await
                .unwrap_or_else(|e| {
                    tracing::warn!(
                        context_id = %context_id,
                        "Failed to notify tool list changed for connection '{}': {}",
                        connection_id,
                        e
                    );
                });
        }

        info!(
            context_id = %context_id,
            "客户端注册完成 | 调用栈: setup_new_client() line {} | 数据流: nvim_clients_after={} | notify_tool_list_changed={}",
            line!(),
            self.nvim_clients.len(),
            should_notify
        );

        Ok(())
    }
}

impl Default for NeovimMcpServer {
    fn default() -> Self {
        Self::new()
    }
}

/// Generate BLAKE3 hash from input string
pub fn b3sum(input: &str) -> String {
    blake3::hash(input.as_bytes()).to_hex().to_string()
}

/// Get git root directory
#[allow(dead_code)]
fn get_git_root() -> Option<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .ok()?;

    if output.status.success() {
        let result = String::from_utf8(output.stdout).ok()?;
        Some(result.trim().to_string())
    } else {
        None
    }
}

/// Get platform-specific temp directory
fn get_temp_dir() -> String {
    if cfg!(target_os = "windows") {
        std::env::var("TEMP").unwrap_or_else(|_| "C:\\temp".to_string())
    } else {
        "/tmp".to_string()
    }
}

#[cfg(unix)]
fn is_live_socket_target(target: &str) -> bool {
    match std::os::unix::net::UnixStream::connect(target) {
        Ok(stream) => {
            drop(stream);
            true
        }
        Err(err) => {
            if matches!(
                err.kind(),
                std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
            ) {
                match std::fs::remove_file(target) {
                    Ok(()) => info!("Removed stale Neovim socket target: {target}"),
                    Err(remove_err) if remove_err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(remove_err) => {
                        warn!("Failed to remove stale Neovim socket {target}: {remove_err}")
                    }
                }
            } else {
                warn!("Skipping unreachable Neovim socket {target}: {err}");
            }

            false
        }
    }
}

#[cfg(not(unix))]
fn is_live_socket_target(_target: &str) -> bool {
    true
}

fn filter_live_socket_targets<I>(targets: I) -> Vec<String>
where
    I: IntoIterator<Item = String>,
{
    targets
        .into_iter()
        .filter(|target| is_live_socket_target(target))
        .collect()
}

/// Find all existing nvim-mcp socket targets in the filesystem
/// Returns a vector of socket paths that match the pattern generated by the Lua plugin
pub fn find_get_all_targets() -> Vec<String> {
    let temp_dir = get_temp_dir();
    let pattern = format!("{temp_dir}/nvim-mcp.*.sock");

    match glob::glob(&pattern) {
        Ok(paths) => filter_live_socket_targets(
            paths
                .filter_map(|entry| entry.ok())
                .map(|path| path.to_string_lossy().to_string()),
        ),
        Err(_) => Vec::new(),
    }
}

/// Get current project root directory
/// Tries git root first, falls back to current working directory
fn get_current_project_root() -> String {
    // Try git root first
    if let Some(git_root) = get_git_root() {
        return git_root;
    }

    // Fallback to current working directory
    std::env::current_dir()
        .unwrap_or_else(|err| {
            warn!("Failed to get current working directory: {}", err);
            std::path::PathBuf::from("<unknown project root>")
        })
        .to_string_lossy()
        .to_string()
}

/// Escape path for use in filename by replacing problematic characters
/// Matches the Lua plugin behavior: replaces '/' with '%'
fn escape_path(path: &str) -> String {
    path.trim().replace("/", "%")
}

/// Find nvim-mcp socket targets for the current project only
/// Returns sockets that match the current project's escaped path
pub fn find_targets_for_current_project() -> Vec<String> {
    let current_project_root = get_current_project_root();
    let escaped_project_root = escape_path(&current_project_root);

    let temp_dir = get_temp_dir();
    let pattern = format!("{temp_dir}/nvim-mcp.{escaped_project_root}.*.sock");

    match glob::glob(&pattern) {
        Ok(paths) => filter_live_socket_targets(
            paths
                .filter_map(|entry| entry.ok())
                .map(|path| path.to_string_lossy().to_string()),
        ),
        Err(e) => {
            warn!(
                "Glob error while searching for Neovim sockets with pattern '{}': {}",
                pattern, e
            );
            Vec::new()
        }
    }
}

/// Connect to a single target and return the connection ID
/// Reusable for both auto-connect and specific target modes
pub async fn auto_connect_single_target(
    server: &NeovimMcpServer,
    target: &str,
) -> Result<String, NeovimError> {
    let connection_id = server.generate_shorter_connection_id(target);

    // Check if already connected (connection replacement logic)
    if let Some(mut old_client) = server.nvim_clients.get_mut(&connection_id) {
        if let Some(existing_target) = old_client.target()
            && existing_target == target
        {
            debug!("Already connected to {target} with ID {connection_id}");
            return Ok(connection_id); // Already connected to same target
        }
        // Different target, disconnect old one
        debug!("Disconnecting old connection for {target}");
        let _ = old_client.disconnect().await;
    }

    // Import NeovimClient here to avoid circular imports
    let mut client = crate::neovim::NeovimClient::default();
    client.connect_path(target).await?;

    server
        .nvim_clients
        .insert(connection_id.clone(), Box::new(client));
    debug!("Successfully connected to {target} with ID {connection_id}");
    Ok(connection_id)
}

/// Auto-connect to all Neovim targets for the current project
/// Returns list of successful connection IDs, or list of failures
pub async fn auto_connect_current_project_targets(
    server: &NeovimMcpServer,
) -> Result<Vec<String>, Vec<(String, String)>> {
    let project_targets = find_targets_for_current_project();
    let current_project = get_current_project_root();

    if project_targets.is_empty() {
        info!("No Neovim instances found for current project: {current_project}");
        return Ok(Vec::new());
    }

    info!(
        "Found {} Neovim instances for current project: {current_project}",
        project_targets.len()
    );

    let mut successful_connections = Vec::new();
    let mut failed_connections = Vec::new();

    for target in project_targets {
        match auto_connect_single_target(server, &target).await {
            Ok(connection_id) => {
                successful_connections.push(connection_id);
                info!("Auto-connected to project Neovim instance: {target}");
            }
            Err(e) => {
                failed_connections.push((target.clone(), e.to_string()));
                warn!("Failed to auto-connect to {target}: {e}");
            }
        }
    }

    if successful_connections.is_empty() && !failed_connections.is_empty() {
        Err(failed_connections)
    } else {
        Ok(successful_connections)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use async_trait::async_trait;
    use rmpv::Value;

    use super::*;
    use crate::neovim::{
        DocumentIdentifier, NeovimError, Position,
        client::{BufferInfo, NavigateResult, Notification},
    };

    struct StubClient {
        target: String,
    }

    #[async_trait]
    impl NeovimClientTrait for StubClient {
        fn target(&self) -> Option<String> {
            Some(self.target.clone())
        }

        async fn disconnect(&mut self) -> Result<String, NeovimError> {
            Ok("disconnected".to_string())
        }

        async fn get_buffers(&self) -> Result<Vec<BufferInfo>, NeovimError> {
            Ok(Vec::new())
        }

        async fn execute_lua(&self, _code: &str) -> Result<Value, NeovimError> {
            Ok(Value::Nil)
        }

        async fn wait_for_notification(
            &self,
            notification_name: &str,
            _timeout_ms: u64,
        ) -> Result<Notification, NeovimError> {
            Ok(Notification {
                name: notification_name.to_string(),
                args: Vec::new(),
                timestamp: std::time::SystemTime::now(),
            })
        }

        async fn navigate(
            &self,
            _document: DocumentIdentifier,
            _position: Position,
        ) -> Result<NavigateResult, NeovimError> {
            Ok(NavigateResult {
                path: self.target.clone(),
                line: 0,
                column: 0,
            })
        }

        async fn read_document(
            &self,
            _document: DocumentIdentifier,
            _start: i64,
            _end: i64,
        ) -> Result<String, NeovimError> {
            Ok(String::new())
        }
    }

    #[test]
    fn test_http_session_server_preserves_existing_connections() {
        let server = NeovimMcpServer::with_connect_mode(Some("auto".to_string()));
        server.nvim_clients.insert(
            "abc1234".to_string(),
            Box::new(StubClient {
                target: "/tmp/project.sock".to_string(),
            }),
        );

        let session_server = server.server_for_http_session();

        assert_eq!(session_server.connect_mode.as_deref(), Some("auto"));
        assert_eq!(session_server.nvim_clients.len(), 1);
        assert!(session_server.get_connection("abc1234").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn test_find_targets_for_current_project_filters_stale_sockets() {
        use std::os::unix::net::UnixListener;

        let suffix = crate::test_utils::generate_random_id();
        let escaped_project_root = escape_path(&get_current_project_root());
        let live_socket = format!("/tmp/nvim-mcp.{escaped_project_root}.{suffix}-live.sock");
        let stale_socket = format!("/tmp/nvim-mcp.{escaped_project_root}.{suffix}-stale.sock");

        let live_listener =
            UnixListener::bind(&live_socket).expect("Should create live test socket");
        let stale_listener =
            UnixListener::bind(&stale_socket).expect("Should create stale test socket");
        drop(stale_listener);

        let matching_targets: Vec<_> = find_targets_for_current_project()
            .into_iter()
            .filter(|target| target.contains(&suffix))
            .collect();

        assert!(
            matching_targets.contains(&live_socket),
            "Live socket should remain discoverable"
        );
        assert!(
            !matching_targets.contains(&stale_socket),
            "Stale socket should be filtered out"
        );
        assert!(
            !Path::new(&stale_socket).exists(),
            "Stale socket file should be cleaned up"
        );

        drop(live_listener);
        let _ = std::fs::remove_file(&live_socket);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn test_auto_connect_current_project_targets_skips_stale_sockets()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::net::UnixListener;

        let _lock = crate::test_utils::NEOVIM_TEST_MUTEX.lock().unwrap();

        let suffix = crate::test_utils::generate_random_id();
        let escaped_project_root = escape_path(&get_current_project_root());
        let live_socket = format!("/tmp/nvim-mcp.{escaped_project_root}.{suffix}-live.sock");
        let stale_socket = format!("/tmp/nvim-mcp.{escaped_project_root}.{suffix}-stale.sock");

        let stale_listener =
            UnixListener::bind(&stale_socket).expect("Should create stale test socket");
        drop(stale_listener);

        let nvim_guard = crate::test_utils::setup_test_neovim_instance(&live_socket).await?;
        let server = NeovimMcpServer::with_connect_mode(Some("auto".to_string()));

        auto_connect_current_project_targets(&server)
            .await
            .expect("Auto-connect should still succeed with a stale socket present");

        let connected_targets: Vec<_> = server
            .nvim_clients
            .iter()
            .filter_map(|entry| entry.value().target())
            .filter(|target| target.contains(&suffix))
            .collect();

        assert!(
            connected_targets.contains(&live_socket),
            "Live socket should be auto-connected"
        );
        assert!(
            !connected_targets.contains(&stale_socket),
            "Stale socket should not be auto-connected"
        );
        assert!(
            !Path::new(&stale_socket).exists(),
            "Stale socket should be removed during auto-discovery"
        );

        drop(nvim_guard);

        Ok(())
    }
}
