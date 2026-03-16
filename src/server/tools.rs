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
            format!("absolute_path={}", preview_text(&path.display().to_string(), 80))
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
        tracing::info!(
            context_id = "get_targets",
            "扫描 Neovim targets | 调用栈: tool_router() → get_targets() line {} | 数据流: 输出 targets={}",
            line!(),
            targets.len()
        );
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
        tracing::info!(
            context_id = %context_id,
            "开始连接 Neovim | 调用栈: request() → connect() line {} | 数据流: 输入 target={} → connection_id={}",
            line!(),
            preview_text(&path, 120),
            connection_id
        );

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_path(&path).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        tracing::info!(
            context_id = %context_id,
            "Neovim 连接成功 | 调用栈: connect() line {} | 数据流: target={} → status=connected",
            line!(),
            preview_text(&path, 120)
        );

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
        tracing::info!(
            context_id = %context_id,
            "开始 TCP 连接 | 调用栈: request() → connect_tcp() line {} | 数据流: 输入 address={} → connection_id={}",
            line!(),
            preview_text(&address, 120),
            connection_id
        );

        // If connection already exists, disconnect the old one first (ignoring errors)
        if let Some(mut old_client) = self.nvim_clients.get_mut(&connection_id) {
            let _ = old_client.disconnect().await;
        }

        let mut client = NeovimClient::default();
        client.connect_tcp(&address).await?;

        self.setup_new_client(&connection_id, Box::new(client), &ctx)
            .await?;

        tracing::info!(
            context_id = %context_id,
            "TCP 连接成功 | 调用栈: connect_tcp() line {} | 数据流: address={} → status=connected",
            line!(),
            preview_text(&address, 120)
        );

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
        tracing::info!(
            context_id = %context_id,
            "开始断开连接 | 调用栈: request() → disconnect() line {} | 数据流: connection_id={} target={}",
            line!(),
            connection_id,
            preview_text(&target, 120)
        );

        // Remove the connection from the map
        if let Some((_, mut client)) = self.nvim_clients.remove(&connection_id) {
            if let Err(e) = client.disconnect().await {
                return Err(McpError::internal_error(
                    format!("Failed to disconnect: {e}"),
                    None,
                ));
            }
            tracing::info!(
                context_id = %context_id,
                "断开连接成功 | 调用栈: disconnect() line {} | 数据流: connection_id={} → status=disconnected",
                line!(),
                connection_id
            );
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
        tracing::info!(
            context_id = %context_id,
            "读取 buffers 成功 | 调用栈: request() → list_buffers() line {} | 数据流: 输入 connection_id={} → 输出 buffers={}",
            line!(),
            connection_id,
            buffers.len()
        );
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
        let document_summary = summarize_document(&document);
        let client = self.get_connection(&connection_id)?;
        let content = client.read_document(document, start, end).await?;
        tracing::info!(
            context_id = %context_id,
            "读取文档成功 | 调用栈: request() → read_document_tool() line {} | 数据流: {} range=[{}, {}) → bytes={}",
            line!(),
            document_summary,
            start,
            end,
            content.len()
        );
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
        tracing::info!(
            context_id = %context_id,
            "执行 Lua | 调用栈: request() → exec_lua() line {} | 数据流: code_bytes={} code_preview={}",
            line!(),
            code.len(),
            preview_text(&code, 120)
        );
        let client = self.get_connection(&connection_id)?;
        let result = client.execute_lua(&code).await?;
        tracing::info!(
            context_id = %context_id,
            "Lua 执行成功 | 调用栈: exec_lua() line {} | 数据流: 输出 result_type={}",
            line!(),
            result.to_string()
        );
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
        tracing::info!(
            context_id = %context_id,
            "读取光标位置成功 | 调用栈: request() → cursor_position() line {} | 数据流: 输出={}",
            line!(),
            json_result
        );

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
        tracing::info!(
            context_id = %context_id,
            "开始导航 | 调用栈: request() → navigate() line {} | 数据流: {} pos=({}, {})",
            line!(),
            summarize_document(&document),
            position.line,
            position.character
        );
        let client = self.get_connection(&connection_id)?;
        let result = client.navigate(document, position).await?;
        tracing::info!(
            context_id = %context_id,
            "导航成功 | 调用栈: navigate() line {} | 数据流: 输出 path={} line={} col={}",
            line!(),
            preview_text(&result.path, 120),
            result.line,
            result.column
        );
        Ok(CallToolResult::success(vec![Content::json(result)?]))
    }
}

/// Build tool router for NeovimMcpServer
pub fn build_tool_router() -> ToolRouter<NeovimMcpServer> {
    NeovimMcpServer::tool_router()
}
