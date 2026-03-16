use rmcp::{
    ErrorData as McpError, ServerHandler,
    model::*,
    service::{RequestContext, RoleServer},
};
use serde_json::json;
use tracing::{debug, info, instrument};

use super::core::NeovimMcpServer;
use crate::logging::{preview_json, preview_text, request_context_id};

fn new_resource(uri: &str, name: &str, description: Option<&str>) -> Resource {
    Resource {
        raw: RawResource {
            uri: uri.to_string(),
            name: name.to_string(),
            description: description.map(|s| s.to_string()),
            mime_type: Some("application/json".to_string()),
            size: None,
            icons: None,
            title: None,
            meta: None,
        },
        annotations: None,
    }
}
// Manual ServerHandler implementation to override tool methods
impl ServerHandler for NeovimMcpServer {
    #[instrument(skip(self))]
    fn get_info(&self) -> ServerInfo {
        info!(
            context_id = "server_info",
            "返回 server info | 调用栈: ServerHandler::get_info() line {} | 数据流: capabilities=tools,tool_list_changed,resources",
            line!()
        );
        ServerInfo {
            instructions: None,
            capabilities: ServerCapabilities::builder()
                .enable_tools()
                .enable_tool_list_changed()
                .enable_resources()
                .build(),
            ..Default::default()
        }
    }

    #[instrument(skip(self))]
    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, McpError> {
        let context_id = request_context_id(&context, "list_resources");
        debug!("Listing available resources");
        info!(
            context_id = %context_id,
            "列出资源 | 调用栈: request() → list_resources() line {} | 数据流: active_connections={}",
            line!(),
            self.nvim_clients.len()
        );

        let mut resources = vec![
            new_resource(
                "nvim-connections://",
                "Active Neovim Connections",
                Some("List of active Neovim connections"),
            ),
            new_resource(
                "nvim-tools://",
                "Tool Registration Overview",
                Some("Overview of all tools and their connection mappings"),
            ),
        ];

        // Add connection-specific resources
        for connection_entry in self.nvim_clients.iter() {
            let connection_id = connection_entry.key().clone();

            // Add connection-specific tools resource
            resources.push(new_resource(
                &format!("nvim-tools://{connection_id}"),
                &format!("Tools for Connection ({connection_id})"),
                Some(&format!(
                    "List of tools available for connection {connection_id}"
                )),
            ));
        }

        Ok(ListResourcesResult {
            resources,
            next_cursor: None,
            meta: None,
        })
    }

    #[instrument(skip(self))]
    async fn read_resource(
        &self,
        ReadResourceRequestParams { uri, .. }: ReadResourceRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResult, McpError> {
        let context_id = request_context_id(&context, "read_resource");
        debug!("Reading resource: {}", uri);
        info!(
            context_id = %context_id,
            "读取资源 | 调用栈: request() → read_resource() line {} | 数据流: uri={}",
            line!(),
            preview_text(uri.as_str(), 120)
        );

        match uri.as_str() {
            "nvim-connections://" => {
                let connections: Vec<_> = self
                    .nvim_clients
                    .iter()
                    .map(|entry| {
                        json!({
                            "id": entry.key(),
                            "target": entry.value().target()
                                .unwrap_or_else(|| "Unknown".to_string())
                        })
                    })
                    .collect();

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&connections).map_err(|e| {
                            McpError::internal_error(
                                "Failed to serialize connections",
                                Some(json!({"error": e.to_string()})),
                            )
                        })?,
                        uri,
                    )],
                })
            }
            "nvim-tools://" => {
                // Overview of all tools and their connection mappings
                let static_tools: Vec<_> = self
                    .hybrid_router
                    .static_router()
                    .list_all()
                    .into_iter()
                    .map(|tool| {
                        json!({
                            "name": tool.name,
                            "description": tool.description,
                            "type": "static",
                            "available_to": "all_connections"
                        })
                    })
                    .collect();

                let mut connection_tools = json!({});
                for connection_entry in self.nvim_clients.iter() {
                    let connection_id = connection_entry.key();
                    let tools_info = self.hybrid_router.get_connection_tools_info(connection_id);
                    let dynamic_tools: Vec<_> = tools_info
                        .into_iter()
                        .filter(|(_, _, is_static)| !is_static) // Only show dynamic tools
                        .map(|(name, description, _)| {
                            json!({
                                "name": name,
                                "description": description,
                                "type": "dynamic"
                            })
                        })
                        .collect();

                    connection_tools[connection_id] = json!(dynamic_tools);
                }

                let overview = json!({
                    "static_tools": static_tools,
                    "connection_specific_tools": connection_tools
                });

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&overview).map_err(|e| {
                            McpError::internal_error(
                                "Failed to serialize tools overview",
                                Some(json!({"error": e.to_string()})),
                            )
                        })?,
                        uri,
                    )],
                })
            }
            uri if uri.starts_with("nvim-tools://") => {
                // Handle connection-specific tool resources like "nvim-tools://{connection_id}"
                let connection_id = uri.strip_prefix("nvim-tools://").unwrap();

                if connection_id.is_empty() {
                    return Err(McpError::invalid_params(
                        "Missing connection ID in tools URI",
                        None,
                    ));
                }

                // Verify connection exists
                let _client = self.get_connection(connection_id)?;

                // Get clean tools info for this connection
                let tools_info_data = self.hybrid_router.get_connection_tools_info(connection_id);
                let tools_info: Vec<_> = tools_info_data
                    .into_iter()
                    .map(|(name, description, is_static)| {
                        json!({
                            "name": name,
                            "description": description,
                            "type": if is_static { "static" } else { "dynamic" },
                            "connection_id": connection_id
                        })
                    })
                    .collect();

                let result = json!({
                    "connection_id": connection_id,
                    "tools": tools_info,
                    "total_count": tools_info.len(),
                    "dynamic_count": self.hybrid_router.get_connection_tool_count(connection_id)
                });

                Ok(ReadResourceResult {
                    contents: vec![ResourceContents::text(
                        serde_json::to_string_pretty(&result).map_err(|e| {
                            McpError::internal_error(
                                "Failed to serialize connection tools",
                                Some(json!({"error": e.to_string()})),
                            )
                        })?,
                        uri,
                    )],
                })
            }
            _ => Err(McpError::resource_not_found(
                "resource_not_found",
                Some(json!({"uri": uri})),
            )),
        }
    }

    // Override list_tools to use HybridToolRouter
    #[instrument(skip(self))]
    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let context_id = request_context_id(&context, "list_tools");
        debug!("Listing tools (static + dynamic) via HybridToolRouter");
        info!(
            context_id = %context_id,
            "列出工具开始 | 调用栈: request() → list_tools() line {} | 数据流: active_connections={} dynamic_tool_names={}",
            line!(),
            self.nvim_clients.len(),
            self.hybrid_router.get_dynamic_tool_count()
        );

        // Get tools from HybridToolRouter instead of static router
        let mut tools = self.hybrid_router.list_all_tools();

        for tool in &mut tools {
            if let Some(extra) = self.get_tool_extra_description(&tool.name) {
                if let Some(desc) = &mut tool.description {
                    // Follow the markdown format, ensuring two new lines between paragraphs
                    let new_desc = format!("{}\n\n{}", desc, extra).trim().to_string();
                    *desc = new_desc.into();
                } else {
                    tool.description = Some(extra.into());
                }
            }
        }

        if self.nvim_clients.is_empty() {
            info!(
                context_id = %context_id,
                "无连接时过滤 connection-aware tools | 调用栈: list_tools() line {} | 数据流: tools_before={}",
                line!(),
                tools.len()
            );
            tools.retain(|tool| {
                !tool
                    .input_schema
                    .get("properties")
                    .map(|x| {
                        if let serde_json::Value::Object(x) = x {
                            x.contains_key("connection_id")
                        } else {
                            false
                        }
                    })
                    .unwrap_or_default()
            });
        }

        info!(
            context_id = %context_id,
            "列出工具完成 | 调用栈: list_tools() line {} | 数据流: tools_after={}",
            line!(),
            tools.len()
        );

        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    // Override call_tool to use HybridToolRouter
    #[instrument(skip(self))]
    async fn call_tool(
        &self,
        CallToolRequestParams {
            name, arguments, ..
        }: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        let context_id = request_context_id(&context, name.as_ref());
        debug!("Calling tool: {} via HybridToolRouter", name);
        info!(
            context_id = %context_id,
            "收到 tool 调用 | 调用栈: request() → ServerHandler::call_tool() line {} | 数据流: tool={} raw_args_present={} request_id={:?} meta={}",
            line!(),
            name,
            arguments.is_some(),
            context.id,
            preview_text(&format!("{:?}", &context.meta), 240)
        );

        // Convert arguments to serde_json::Value
        let args = arguments.unwrap_or_default();
        let args_value = serde_json::to_value(args).map_err(|e| {
            McpError::invalid_params(
                "Failed to serialize arguments",
                Some(json!({"error": e.to_string()})),
            )
        })?;
        info!(
            context_id = %context_id,
            "tool 参数已序列化 | 调用栈: ServerHandler::call_tool() line {} | 数据流: tool={} args={}",
            line!(),
            name,
            preview_json(&args_value, 240)
        );

        // Use HybridToolRouter for dispatch
        let result = self
            .hybrid_router
            .call_tool(self, &name, args_value, context)
            .await;
        match &result {
            Ok(call_result) => info!(
                context_id = %context_id,
                "tool 调用完成 | 调用栈: ServerHandler::call_tool() line {} | 数据流: tool={} is_error={:?} content_items={}",
                line!(),
                name,
                call_result.is_error,
                call_result.content.len()
            ),
            Err(error) => info!(
                context_id = %context_id,
                "tool 调用失败 | 调用栈: ServerHandler::call_tool() line {} | 数据流: tool={} error={}",
                line!(),
                name,
                preview_text(&error.to_string(), 180)
            ),
        }
        result
    }
}
