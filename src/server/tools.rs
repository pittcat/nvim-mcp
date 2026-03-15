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
use crate::neovim::{DocumentIdentifier, NeovimClient, Position, string_or_struct};

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

macro_rules! include_files {
    ($($key:ident),* $(,)?) => {{
        let mut map = HashMap::new();
        $(
            map.insert(stringify!($key), include_str!(concat!("../../docs/tools/", stringify!($key), ".md")));
        )*
        map
    }};
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
        let connection_id = self.generate_shorter_connection_id(&path);

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_path(&path).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({
                "connection_id": connection_id,
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
        let connection_id = self.generate_shorter_connection_id(&address);

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_tcp(&address).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        Ok(CallToolResult::success(vec![Content::json(
            serde_json::json!({
                "connection_id": connection_id,
            }),
        )?]))
    }

    #[tool(description = "Disconnect from Neovim instance")]
    #[instrument(skip(self))]
    pub async fn disconnect(
        &self,
        Parameters(ConnectionRequest { connection_id }): Parameters<ConnectionRequest>,
    ) -> Result<CallToolResult, McpError> {
        // Verify connection exists first
        let target = {
            let client = self.get_connection(&connection_id)?;
            client.target().unwrap_or_else(|| "Unknown".to_string())
        };

        // Remove the connection from the map
        if let Some((_, mut client)) = self.nvim_clients.remove(&connection_id) {
            if let Err(e) = client.disconnect().await {
                return Err(McpError::internal_error(
                    format!("Failed to disconnect: {e}"),
                    None,
                ));
            }
            Ok(CallToolResult::success(vec![Content::json(
                serde_json::json!({
                    "connection_id": connection_id,
                    "target": target,
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
        let client = self.get_connection(&connection_id)?;
        let buffers = client.get_buffers().await?;
        Ok(CallToolResult::success(vec![Content::json(buffers)?]))
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
        let client = self.get_connection(&connection_id)?;
        let result = client.execute_lua(&code).await?;
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
        let client = self.get_connection(&connection_id)?;
        let result = client.navigate(document, position).await?;
        Ok(CallToolResult::success(vec![Content::json(result)?]))
    }
}

/// Build tool router for NeovimMcpServer
pub fn build_tool_router() -> ToolRouter<NeovimMcpServer> {
    NeovimMcpServer::tool_router()
}
