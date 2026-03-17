use std::collections::HashMap;

use rmcp::{
    ErrorData as McpError, RoleServer,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::*,
    schemars,
    service::RequestContext,
    tool, tool_router,
};
use tracing::instrument;

use super::core::{NeovimMcpServer, find_get_all_targets};
use super::lua_tools;
use crate::{
    logging::{connection_context_id, preview_text, request_context_id},
    neovim::{DocumentIdentifier, NeovimClient, Position, string_or_struct},
};

/// Check if the given address looks like a Unix socket path that should not be used with connect_tcp
/// This helps users who mistakenly try to use Unix socket paths with the TCP connection tool.
fn is_unix_socket_path(address: &str) -> bool {
    // Common patterns for Unix socket paths:
    // - Absolute paths starting with / (e.g., /tmp/nvim.sock)
    // - Paths containing .sock extension
    // - Paths that look like typical socket file locations

    // Check if it's clearly a Unix socket path
    if address.starts_with('/') {
        // It's an absolute path - likely a Unix socket
        // But also check if it could be a legitimate path-style TCP address on Linux
        // Linux abstract socket addresses start with @

        // If it starts with / and doesn't contain : (which would indicate TCP host:port)
        // and it's not an IPv6 address, it's likely a Unix socket
        if !address.contains(':') && !address.starts_with("/[") {
            return true;
        }
    }

    // Check for common socket file extensions
    if address.ends_with(".sock") || address.contains("/nvim") && address.contains(".sock") {
        return true;
    }

    // Check for typical socket directory patterns
    if address.starts_with("/tmp/")
        || address.starts_with("/var/run/")
        || address.starts_with("/usr/local/")
    {
        // Further check: if no port number (no colon with digits), likely a socket
        if !address.matches(':').count() >= 2 {
            return true;
        }
    }

    false
}

/// Connect to Neovim instance via unix socket or TCP
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectNvimRequest {
    /// target can be a unix socket path or a TCP address
    pub target: String,
}

/// New parameter struct for connection-aware requests
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ConnectionRequest {
    /// Unique identifier for the target Neovim instance
    pub connection_id: String,
}

/// Lua execution request
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ExecuteLuaRequest {
    /// Unique identifier for the target Neovim instance
    pub connection_id: String,
    /// Lua code to execute in Neovim
    pub code: String,
}

/// Navigate to a specific position in the current buffer or open a file at a specific position
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct NavigateParams {
    /// Unique identifier for the target Neovim instance
    pub connection_id: String,
    /// Document to navigate to
    // Supports both string and struct deserialization.
    // Compatible with Claude Code when using subscription.
    #[serde(deserialize_with = "string_or_struct")]
    pub document: DocumentIdentifier,
    /// Symbol position (zero-based)
    #[serde(flatten)]
    pub position: Position,
}

fn default_read_end() -> i64 {
    -1
}

/// Read document content by buffer ID or file path
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ReadDocumentRequest {
    /// Unique identifier for the target Neovim instance
    pub connection_id: String,
    /// Document to read
    #[serde(deserialize_with = "string_or_struct")]
    pub document: DocumentIdentifier,
    /// Start line index (0-based, inclusive)
    #[serde(default)]
    pub start: i64,
    /// End line index (0-based, exclusive). -1 means to the end of the document.
    #[serde(default = "default_read_end")]
    pub end: i64,
}

macro_rules! include_files {
    ($($key:ident),* $(,)?) => {{
        let mut map = HashMap::new();
        $(
            map.insert(stringify!($key), include_str!(concat!("../../docs/tools/", stringify!($key), ".md")));
        )*
        map
    }};
}

fn summarize_document(document: &DocumentIdentifier) -> String {
    match document {
        DocumentIdentifier::BufferId(buffer_id) => format!("buffer_id={buffer_id}"),
        DocumentIdentifier::ProjectRelativePath(path) => {
            format!(
                "project_relative_path={}",
                preview_text(&path.display().to_string(), 80)
            )
        }
        DocumentIdentifier::AbsolutePath(path) => {
            format!(
                "absolute_path={}",
                preview_text(&path.display().to_string(), 80)
            )
        }
    }
}

impl NeovimMcpServer {
    pub fn tool_descriptions() -> HashMap<&'static str, &'static str> {
        include_files! {
            get_targets,
            connect,
            read,
        }
    }

    pub fn get_tool_extra_description(&self, name: &str) -> Option<String> {
        if name == "get_targets" {
            Some(self.get_connections_instruction())
        } else {
            None
        }
    }
}

#[tool_router]
impl NeovimMcpServer {
    #[tool]
    #[instrument(skip(self))]
    pub async fn get_targets(&self) -> Result<CallToolResult, McpError> {
        let targets = find_get_all_targets();
        tracing::debug!(context_id = "get_targets", "Found {} targets", targets.len());
        if targets.is_empty() {
            return Err(McpError::invalid_request(
                "No Neovim targets found".to_string(),
                None,
            ));
        }

        Ok(CallToolResult::success(vec![Content::json(targets)?]))
    }

    #[tool]
    #[instrument(skip(self))]
    pub async fn connect(
        &self,
        Parameters(ConnectNvimRequest { target: path }): Parameters<ConnectNvimRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = request_context_id(&ctx, "connect");
        let connection_id = self.generate_shorter_connection_id(&path);
        tracing::debug!(context_id = %context_id, "Connecting to: {}", preview_text(&path, 120));

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_path(&path).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        tracing::debug!(context_id = %context_id, "Connected successfully");

        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({
                "connection_id": connection_id,
                "status": "connected",
            }),
        )?]))
    }

    #[tool(description = "Connect via TCP address")]
    #[instrument(skip(self))]
    pub async fn connect_tcp(
        &self,
        Parameters(ConnectNvimRequest { target: address }): Parameters<ConnectNvimRequest>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = request_context_id(&ctx, "connect_tcp");
        let connection_id = self.generate_shorter_connection_id(&address);
        tracing::debug!(context_id = %context_id, "Connecting via TCP to: {}", preview_text(&address, 120));

        // Check if the address looks like a Unix socket path (common misuse)
        if is_unix_socket_path(&address) {
            tracing::warn!(
                context_id = %context_id,
                "Received Unix socket path for TCP connection: {}",
                preview_text(&address, 120)
            );
            return Err(McpError::invalid_params(
                format!(
                    "The address '{}' appears to be a Unix socket path. \
                     Please use the 'connect' tool instead of 'connect_tcp' for Unix socket connections.",
                    address
                ),
                Some(serde_json::json!({
                    "address": address,
                    "suggested_tool": "connect",
                    "reason": "Unix socket paths should use the 'connect' tool"
                })),
            ));
        }

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_tcp(&address).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        tracing::debug!(context_id = %context_id, "TCP connection successful");

        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({
                "connection_id": connection_id,
                "status": "connected",
            }),
        )?]))
    }

    #[tool(description = "Disconnect from Neovim instance")]
    #[instrument(skip(self))]
    pub async fn disconnect(
        &self,
        Parameters(ConnectionRequest { connection_id }): Parameters<ConnectionRequest>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "disconnect");
        // Verify connection exists first
        let target = {
            let client = self.get_connection(&connection_id)?;
            client.target().unwrap_or_else(|| "Unknown".to_string())
        };
        tracing::debug!(context_id = %context_id, "Disconnecting from: {}", preview_text(&target, 120));

        // Remove the connection from the map
        if let Some((_, mut client)) = self.nvim_clients.remove(&connection_id) {
            if let Err(e) = client.disconnect().await {
                return Err(McpError::internal_error(
                    format!("Failed to disconnect: {e}"),
                    None,
                ));
            }
            tracing::debug!(context_id = %context_id, "Disconnected successfully");
            Ok(CallToolResult::success(vec![Content::json(
                serde_json::json!({
                    "connection_id": connection_id,
                    "target": target,
                    "status": "disconnected",
                }),
            )?]))
        } else {
            Err(McpError::invalid_request(
                format!("No Neovim connection found for ID: {connection_id}"),
                None,
            ))
        }
    }

    #[tool(description = "List all open buffers")]
    #[instrument(skip(self))]
    pub async fn list_buffers(
        &self,
        Parameters(ConnectionRequest { connection_id }): Parameters<ConnectionRequest>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "list_buffers");
        let client = self.get_connection(&connection_id)?;
        let buffers = client.get_buffers().await?;
        tracing::debug!(context_id = %context_id, "Listed {} buffers", buffers.len());
        Ok(CallToolResult::success(vec![Content::json(buffers)?]))
    }

    #[tool(
        name = "read",
        description = "Read document content with universal document identification"
    )]
    #[instrument(skip(self))]
    pub async fn read_document_tool(
        &self,
        Parameters(ReadDocumentRequest {
            connection_id,
            document,
            start,
            end,
        }): Parameters<ReadDocumentRequest>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "read");
        let client = self.get_connection(&connection_id)?;
        let content = client.read_document(document, start, end).await?;
        tracing::debug!(context_id = %context_id, "Read document: {} bytes", content.len());
        Ok(CallToolResult::success(vec![Content::text(content)]))
    }

    #[tool(description = "Execute Lua code")]
    #[instrument(skip(self))]
    pub async fn exec_lua(
        &self,
        Parameters(ExecuteLuaRequest {
            connection_id,
            code,
        }): Parameters<ExecuteLuaRequest>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "exec_lua");
        tracing::debug!(context_id = %context_id, "Executing Lua: {}", preview_text(&code, 120));
        let client = self.get_connection(&connection_id)?;
        let result = client.execute_lua(&code).await?;
        tracing::debug!(context_id = %context_id, "Lua executed successfully");
        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({
                "result": format!("{:?}", result)
            }),
        )?]))
    }

    #[tool(
        description = "Get the current cursor position: buffer id, buffer name, window id, and zero-based row/col index"
    )]
    #[instrument(skip(self))]
    pub async fn cursor_position(
        &self,
        Parameters(ConnectionRequest { connection_id }): Parameters<ConnectionRequest>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "cursor_position");
        let client = self.get_connection(&connection_id)?;
        let lua_code = include_str!("./lua/cursor_position.lua");
        let result = client.execute_lua(lua_code).await?;

        // Convert nvim Value to serde_json::Value for serialization
        let json_result = lua_tools::convert_nvim_value_to_json(result).map_err(|e| {
            McpError::internal_error(
                format!("Failed to convert cursor position result to JSON: {}", e),
                None,
            )
        })?;
        tracing::debug!(context_id = %context_id, "Cursor position retrieved");

        Ok(CallToolResult::success(vec![Content::json(json_result)?]))
    }

    #[tool(
        description = "Navigate to a specific position in the current buffer or open a file at a specific position"
    )]
    #[instrument(skip(self))]
    pub async fn navigate(
        &self,
        Parameters(NavigateParams {
            connection_id,
            document,
            position,
        }): Parameters<NavigateParams>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = connection_context_id(&connection_id, "navigate");
        tracing::debug!(context_id = %context_id, "Navigating to {} at line {}, char {}",
            summarize_document(&document), position.line, position.character);
        let client = self.get_connection(&connection_id)?;
        let result = client.navigate(document, position).await?;
        tracing::debug!(context_id = %context_id, "Navigation complete: {}", preview_text(&result.path, 120));
        Ok(CallToolResult::success(vec![Content::json(result)?]))
    }
}

/// Build tool router for NeovimMcpServer
pub fn build_tool_router() -> ToolRouter<NeovimMcpServer> {
    NeovimMcpServer::tool_router()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_unix_socket_path() {
        // Unix socket paths should be detected
        assert!(is_unix_socket_path("/tmp/nvim.sock"));
        assert!(is_unix_socket_path("/tmp/nvim-mcp.test.sock"));
        assert!(is_unix_socket_path("/var/run/nvim.sock"));
        assert!(is_unix_socket_path("/usr/local/bin/nvim.sock"));

        // TCP addresses should not be detected as Unix sockets
        assert!(!is_unix_socket_path("127.0.0.1:6666"));
        assert!(!is_unix_socket_path("localhost:8080"));
        assert!(!is_unix_socket_path("192.168.1.1:9000"));
        assert!(!is_unix_socket_path("[::1]:6666"));
    }
}
