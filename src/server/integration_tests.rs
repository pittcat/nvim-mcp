use rmcp::{
    model::CallToolRequestParams,
    serde_json::{Map, Value},
    service::ServiceExt,
    transport::{ConfigureCommandExt, TokioChildProcess},
};
use tokio::process::Command;
use tracing::{error, info};
use tracing_test::traced_test;

use crate::test_utils::*;

/// Helper function to create CallToolRequestParams with only required fields.
/// Other fields use default values to avoid API breakage when rmcp adds new fields.
fn call_tool_req(
    name: impl Into<String>,
    arguments: Option<Map<String, Value>>,
) -> CallToolRequestParams {
    CallToolRequestParams {
        name: name.into().into(),
        arguments,
        meta: None,
        task: None,
    }
}

// Macro to create an MCP service using the pre-compiled binary
macro_rules! create_mcp_service {
    () => {{
        let command = Command::new(get_compiled_binary()).configure(|cmd| {
            cmd.args(["--connect", "manual"]);
        });
        ().serve(TokioChildProcess::new(command)?)
            .await
            .map_err(|e| {
                error!("Failed to connect to server: {}", e);
                e
            })?
    }};
    ($target:expr) => {{
        // Macro to create an MCP service with auto-connect to a specific target
        let command = Command::new(get_compiled_binary()).configure(|cmd| {
            cmd.args(["--connect", $target]);
        });
        ().serve(TokioChildProcess::new(command)?)
            .await
            .map_err(|e| {
                error!("Failed to connect to server: {}", e);
                e
            })?
    }};
}

// Macro to set up a connected MCP service (service + connection_id + guard)
macro_rules! setup_connected_service {
    () => {{
        let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

        // Generate random socket path for test isolation
        let ipc_path = generate_random_ipc_path();
        let nvim_guard = setup_test_neovim_instance(&ipc_path).await?;

        // Create MCP service
        let service = create_mcp_service!();

        // Connect to the Neovim instance
        let mut connect_args = Map::new();
        connect_args.insert("target".to_string(), Value::String(ipc_path.clone()));

        let result = service
            .call_tool(call_tool_req("connect", Some(connect_args)))
            .await?;

        // Extract connection_id from JSON result
        let content_text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.clone())
            .unwrap_or_default();

        let response_json: serde_json::Value = serde_json::from_str(&content_text)?;
        let connection_id = response_json["connection_id"]
            .as_str()
            .ok_or("Failed to extract connection_id from response")?
            .to_string();

        (service, connection_id, nvim_guard)
    }};
}

#[tokio::test]
#[traced_test]
async fn test_mcp_server_connection() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing MCP server connection");

    // Acquire global lock to prevent concurrent port/socket conflicts
    let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

    // Generate random socket path for test isolation
    let ipc_path = generate_random_ipc_path();
    let nvim_guard = setup_test_neovim_instance(&ipc_path).await?;

    // Create MCP service
    let service = create_mcp_service!();

    // Test connection
    let mut connect_args = Map::new();
    connect_args.insert("target".to_string(), Value::String(ipc_path.clone()));

    let result = service
        .call_tool(call_tool_req("connect", Some(connect_args)))
        .await?;

    // Verify response
    assert!(!result.content.is_empty());
    let content = result.content.first().unwrap().as_text().unwrap();
    assert!(
        content.text.contains("connection_id"),
        "Expected connection_id in response, got: {}",
        content.text
    );

    info!("Connection test result: {}", content.text);

    // Clean up
    service.cancel().await?;
    drop(nvim_guard);

    info!("MCP server connection test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_connect_nvim() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing connect tool");

    let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

    // Generate random socket path for test isolation
    let ipc_path = generate_random_ipc_path();
    let nvim_guard = setup_test_neovim_instance(&ipc_path).await?;

    // Create MCP service
    let service = create_mcp_service!();

    // Test connection via tool
    let mut connect_args = Map::new();
    connect_args.insert("target".to_string(), Value::String(ipc_path.clone()));

    let result = service
        .call_tool(call_tool_req("connect", Some(connect_args)))
        .await?;

    // Verify response - connection returns JSON with connection_id
    assert!(!result.content.is_empty());
    let content = result.content.first().unwrap().as_text().unwrap();
    assert!(
        content.text.contains("connection_id"),
        "Expected connection_id in response, got: {}",
        content.text
    );

    // Parse connection_id from JSON response
    let response_json: serde_json::Value = serde_json::from_str(&content.text)?;
    let connection_id = response_json["connection_id"]
        .as_str()
        .ok_or("Failed to extract connection_id from response")?;

    info!("Connected with ID: {}", connection_id);

    // Verify connection exists via list_buffers
    let mut list_args = Map::new();
    list_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.to_string()),
    );

    let buffers_result = service
        .call_tool(call_tool_req("list_buffers", Some(list_args)))
        .await?;

    assert!(!buffers_result.content.is_empty());

    service.cancel().await?;
    info!("Connect tool test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_disconnect_nvim() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing disconnect tool");

    let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

    // Generate random socket path for test isolation
    let ipc_path = generate_random_ipc_path();
    let nvim_guard = setup_test_neovim_instance(&ipc_path).await?;

    // Create MCP service
    let service = create_mcp_service!();

    // First connect
    let mut connect_args = Map::new();
    connect_args.insert("target".to_string(), Value::String(ipc_path.clone()));

    let result = service
        .call_tool(call_tool_req("connect", Some(connect_args)))
        .await?;

    // Extract connection_id from JSON result
    let content_text = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();

    let response_json: serde_json::Value = serde_json::from_str(&content_text)?;
    let connection_id = response_json["connection_id"]
        .as_str()
        .ok_or("Failed to extract connection_id from response")?
        .to_string();

    info!("Connected with ID: {}", connection_id);

    // Then disconnect
    let mut disconnect_args = Map::new();
    disconnect_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.clone()),
    );

    let result = service
        .call_tool(call_tool_req("disconnect", Some(disconnect_args)))
        .await?;

    assert!(!result.content.is_empty());
    let content = result.content.first().unwrap().as_text().unwrap();
    assert!(
        content.text.contains("disconnected") || content.text.contains("success"),
        "Expected disconnect success, got: {}",
        content.text
    );

    info!("Disconnect result: {}", content.text);

    // Verify connection no longer exists
    let mut list_args = Map::new();
    list_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.to_string()),
    );

    let result = service
        .call_tool(call_tool_req("list_buffers", Some(list_args)))
        .await;

    assert!(
        result.is_err(),
        "Expected error for disconnected connection"
    );

    service.cancel().await?;
    info!("Disconnect tool test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_error_handling() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing error handling");

    let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

    // Create service without any connections
    let service = create_mcp_service!();

    // Try to list buffers without a valid connection
    let mut list_args = Map::new();
    list_args.insert(
        "connection_id".to_string(),
        Value::String("invalid_connection_id".to_string()),
    );

    let result = service
        .call_tool(call_tool_req("list_buffers", Some(list_args)))
        .await;

    assert!(result.is_err(), "Expected error for invalid connection");
    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("No Neovim connection found"),
        "Error should indicate connection not found: {}",
        error
    );

    service.cancel().await?;
    info!("Error handling test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_invalid_connection_id_handling() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing invalid connection ID handling");

    let _guard = NEOVIM_TEST_MUTEX.lock().unwrap();

    // Create two separate Neovim instances
    let ipc_path1 = generate_random_ipc_path();
    let ipc_path2 = generate_random_ipc_path();

    let nvim_guard1 = setup_test_neovim_instance(&ipc_path1).await?;
    let nvim_guard2 = setup_test_neovim_instance(&ipc_path2).await?;

    // Create MCP service
    let service = create_mcp_service!();

    // Connect to first instance
    let mut connect_args = Map::new();
    connect_args.insert("target".to_string(), Value::String(ipc_path1.clone()));

    let result1 = service
        .call_tool(call_tool_req("connect", Some(connect_args)))
        .await?;

    let content1_text = result1
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();
    let response_json1: serde_json::Value = serde_json::from_str(&content1_text)?;
    let connection_id1 = response_json1["connection_id"]
        .as_str()
        .ok_or("Failed to extract connection_id1")?
        .to_string();

    // Connect to second instance
    let mut connect_args = Map::new();
    connect_args.insert("target".to_string(), Value::String(ipc_path2.clone()));

    let result2 = service
        .call_tool(call_tool_req("connect", Some(connect_args)))
        .await?;

    let content2_text = result2
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();
    let response_json2: serde_json::Value = serde_json::from_str(&content2_text)?;
    let connection_id2 = response_json2["connection_id"]
        .as_str()
        .ok_or("Failed to extract connection_id2")?
        .to_string();

    info!(
        "Connected to two instances: {} and {}",
        connection_id1, connection_id2
    );

    // Verify we can access both connections
    let mut list_args1 = Map::new();
    list_args1.insert(
        "connection_id".to_string(),
        Value::String(connection_id1.clone()),
    );

    let buffers1 = service
        .call_tool(call_tool_req("list_buffers", Some(list_args1)))
        .await?;
    assert!(!buffers1.content.is_empty());

    let mut list_args2 = Map::new();
    list_args2.insert(
        "connection_id".to_string(),
        Value::String(connection_id2.clone()),
    );

    let buffers2 = service
        .call_tool(call_tool_req("list_buffers", Some(list_args2)))
        .await?;
    assert!(!buffers2.content.is_empty());

    // Now test using wrong connection ID
    let mut wrong_args = Map::new();
    wrong_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id1.clone()),
    );

    // Use a tool that should fail with connection_id from wrong instance
    // This tests that we properly isolate connections
    let result = service
        .call_tool(call_tool_req("list_buffers", Some(wrong_args)))
        .await;

    // Should succeed because connection_id1 is valid, just a different connection
    assert!(result.is_ok(), "Should be able to use valid connection_id");

    // Now test with completely invalid ID
    let mut invalid_args = Map::new();
    invalid_args.insert(
        "connection_id".to_string(),
        Value::String("completely_invalid_id".to_string()),
    );

    let result = service
        .call_tool(call_tool_req("list_buffers", Some(invalid_args)))
        .await;

    assert!(
        result.is_err(),
        "Expected error for completely invalid connection ID"
    );

    service.cancel().await?;
    info!("Invalid connection ID handling test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_complete_workflow() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing complete workflow");

    let (service, connection_id, _guard) = setup_connected_service!();

    // List buffers
    let mut list_args = Map::new();
    list_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.clone()),
    );

    let result = service
        .call_tool(call_tool_req("list_buffers", Some(list_args)))
        .await?;

    info!("list_buffers result: {:?}", result);
    assert!(!result.content.is_empty());

    // Execute Lua
    let mut lua_args = Map::new();
    lua_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.clone()),
    );
    lua_args.insert(
        "code".to_string(),
        Value::String("return vim.fn.getcwd()".to_string()),
    );

    let result = service
        .call_tool(call_tool_req("exec_lua", Some(lua_args)))
        .await?;

    info!("exec_lua result: {:?}", result);
    assert!(!result.content.is_empty());

    // Get cursor position
    let mut cursor_args = Map::new();
    cursor_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.clone()),
    );

    let result = service
        .call_tool(call_tool_req("cursor_position", Some(cursor_args)))
        .await?;

    info!("cursor_position result: {:?}", result);
    assert!(!result.content.is_empty());

    service.cancel().await?;
    info!("Complete workflow test completed successfully");

    Ok(())
}

#[tokio::test]
#[traced_test]
async fn test_cursor_position_tool() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing cursor_position tool");

    let (service, connection_id, _guard) = setup_connected_service!();

    // Get cursor position
    let mut cursor_args = Map::new();
    cursor_args.insert(
        "connection_id".to_string(),
        Value::String(connection_id.clone()),
    );

    let result = service
        .call_tool(call_tool_req("cursor_position", Some(cursor_args)))
        .await?;

    info!("cursor_position result: {:?}", result);
    assert!(!result.content.is_empty());

    // Parse the result to verify structure
    let content = result
        .content
        .first()
        .and_then(|c| c.as_text())
        .map(|t| t.text.clone())
        .unwrap_or_default();

    info!("Cursor position response: {}", content);

    // Response should contain buffer or position information
    assert!(
        content.contains("buffer") || content.contains("Buffer") || content.contains("position"),
        "Response should contain cursor/buffer information"
    );

    service.cancel().await?;
    info!("Cursor position tool test completed successfully");

    Ok(())
}

// =============================================================================
// HTTP Multi-Client Session Stability Tests
// =============================================================================

/// Helper to send an HTTP POST request to the MCP server
async fn http_post(
    client: &reqwest::Client,
    url: &str,
    body: String,
) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    let response = client
        .post(url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .body(body)
        .send()
        .await?;
    Ok(response)
}

/// Helper to send an HTTP GET request to open an SSE stream
async fn http_get_sse(
    client: &reqwest::Client,
    url: &str,
    session_id: Option<&str>,
) -> Result<reqwest::Response, Box<dyn std::error::Error>> {
    let mut request = client.get(url).header("Accept", "text/event-stream");

    if let Some(sid) = session_id {
        request = request.header("mcp-session-id", sid);
    }

    let response = request.send().await?;
    Ok(response)
}

/// Parse SSE response body into events
fn parse_sse_events(body: &str) -> Vec<String> {
    body.split("\n\n")
        .filter(|e| !e.is_empty())
        .map(|e| e.to_string())
        .collect()
}

/// Setup HTTP server with a test Neovim instance
async fn setup_http_server_with_nvim(
    port: u16,
) -> Result<
    (
        tokio::task::JoinHandle<()>,
        crate::test_utils::NeovimIpcGuard,
        String,
    ),
    Box<dyn std::error::Error>,
> {
    use hyper_util::{
        rt::{TokioExecutor, TokioIo},
        server::conn::auto::Builder,
        service::TowerToHyperService,
    };
    use rmcp::transport::{
        StreamableHttpServerConfig, StreamableHttpService,
        streamable_http_server::session::local::{LocalSessionManager, SessionConfig},
    };
    use tokio::net::TcpListener;

    // Start a Neovim instance
    let ipc_path = crate::test_utils::generate_random_ipc_path();
    let nvim_guard = crate::test_utils::setup_test_neovim_instance(&ipc_path).await?;

    // Create the MCP server with manual connect mode
    let server = crate::NeovimMcpServer::with_connect_mode(Some("manual".to_string()));

    // Connect to the Neovim instance
    let connection_id = server.generate_shorter_connection_id(&ipc_path);
    let mut client = crate::neovim::NeovimClient::default();
    client.connect_path(&ipc_path).await?;
    server
        .nvim_clients
        .insert(connection_id.clone(), Box::new(client));

    // Start HTTP server with configured session manager for multi-client stability
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await?;

    // Configure session manager with increased channel capacity for multi-client support
    let session_config = SessionConfig {
        channel_capacity: 64,
        keep_alive: None,
    };
    let session_manager = LocalSessionManager {
        sessions: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        session_config,
    };

    let server_for_handler = server.clone();
    let handle = tokio::spawn(async move {
        let service = TowerToHyperService::new(StreamableHttpService::new(
            move || Ok(server_for_handler.server_for_http_session()),
            session_manager.into(),
            StreamableHttpServerConfig {
                stateful_mode: true,
                ..Default::default()
            },
        ));

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let service = service.clone();

            tokio::spawn(async move {
                if let Err(e) = Builder::new(TokioExecutor::default())
                    .serve_connection(io, service)
                    .await
                {
                    tracing::error!("HTTP connection error: {}", e);
                }
            });
        }
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    Ok((handle, nvim_guard, connection_id))
}

/// Test that multiple HTTP clients can create/resume sessions without hitting closed channels
#[tokio::test]
#[traced_test]
async fn test_http_multi_client_session_resume_stability() -> Result<(), Box<dyn std::error::Error>>
{
    info!("Testing HTTP multi-client session resume stability");

    // Use a unique port to avoid conflicts
    let port = PORT_BASE + 100;
    let base_url = format!("http://127.0.0.1:{}", port);
    let mcp_url = format!("{}/mcp", base_url);

    // Setup HTTP server with Neovim
    let (_server_handle, _nvim_guard, _connection_id) = setup_http_server_with_nvim(port).await?;

    let client = reqwest::Client::new();

    // Client A: Create session via initialize
    let init_request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test-client-a","version":"1.0"}}}"#.to_string();
    let response = http_post(&client, &mcp_url, init_request).await?;

    assert_eq!(response.status(), 200, "Initialize should succeed");

    // Get session ID from headers
    let session_id_a = response
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s: &str| s.to_string())
        .expect("Should have session ID");

    info!("Client A session ID: {}", session_id_a);

    // Parse SSE response for initialize
    let body = response.text().await?;
    let events = parse_sse_events(&body);
    assert!(!events.is_empty(), "Should receive SSE events");

    // Verify we got the initialize response
    let init_event = events
        .iter()
        .find(|e: &&String| e.contains("\"id\":1"))
        .expect("Should have init response");
    assert!(
        init_event.contains("\"result\""),
        "Should have result in init response"
    );

    // Client B: Create new session (simulating another Claude Code)
    let init_request_b = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test-client-b","version":"1.0"}}}"#.to_string();
    let response_b = http_post(&client, &mcp_url, init_request_b).await?;

    assert_eq!(
        response_b.status(),
        200,
        "Client B initialize should succeed"
    );

    let session_id_b = response_b
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s: &str| s.to_string())
        .expect("Should have session ID for client B");

    info!("Client B session ID: {}", session_id_b);
    assert_ne!(session_id_a, session_id_b, "Sessions should be different");

    // Client A: Send initialized notification (required by MCP protocol)
    // The notification should return 202 Accepted (no response body for notifications)
    let initialized_request =
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
    let response_a_init = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_a)
        .body(initialized_request)
        .send()
        .await?;
    assert_eq!(
        response_a_init.status(),
        202,
        "Initialized notification should return 202"
    );
    info!("Client A sent initialized notification");

    // Client B: Send initialized notification
    let initialized_request =
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
    let response_b_init = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_b)
        .body(initialized_request)
        .send()
        .await?;
    assert_eq!(
        response_b_init.status(),
        202,
        "Initialized notification should return 202"
    );
    info!("Client B sent initialized notification");

    // Small delay to ensure notifications are processed
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Client A: Call tools/list using session
    let tools_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string();
    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_a)
        .body(tools_request)
        .send()
        .await?;

    assert_eq!(response.status(), 200, "Client A tools/list should succeed");
    let body: String = response.text().await?;
    assert!(body.contains("tools"), "Response should contain tools");
    info!("Client A successfully listed tools");

    // Client B: Call tools/list using different session
    let tools_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string();
    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_b)
        .body(tools_request)
        .send()
        .await?;

    assert_eq!(response.status(), 200, "Client B tools/list should succeed");
    let body: String = response.text().await?;
    assert!(body.contains("tools"), "Response should contain tools");
    info!("Client B successfully listed tools");

    // Client C: Create third session
    let init_request_c = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test-client-c","version":"1.0"}}}"#.to_string();
    let response_c = http_post(&client, &mcp_url, init_request_c).await?;

    assert_eq!(
        response_c.status(),
        200,
        "Client C initialize should succeed"
    );

    let session_id_c = response_c
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s: &str| s.to_string())
        .expect("Should have session ID for client C");

    info!("Client C session ID: {}", session_id_c);

    // Client C: Send initialized notification
    let initialized_request =
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
    let response_c_init = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_c)
        .body(initialized_request)
        .send()
        .await?;
    assert_eq!(
        response_c_init.status(),
        202,
        "Initialized notification should return 202"
    );
    info!("Client C sent initialized notification");

    // Small delay to ensure notification is processed
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Verify all three sessions are independent
    let tools_request = r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string();
    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_c)
        .body(tools_request)
        .send()
        .await?;

    assert_eq!(response.status(), 200, "Client C tools/list should succeed");
    info!("Client C successfully listed tools");

    // Verify no "Channel closed" errors in any responses
    let body: String = response.text().await?;
    assert!(
        !body.contains("Channel closed"),
        "Response should not contain 'Channel closed' error: {}",
        body
    );

    info!("Multi-client session resume test completed successfully - no 'Channel closed' errors");

    Ok(())
}

/// Test that connections are consistently visible across multiple sessions
#[tokio::test]
#[traced_test]
async fn test_http_multi_client_shared_connection_visibility()
-> Result<(), Box<dyn std::error::Error>> {
    info!("Testing HTTP multi-client shared connection visibility");

    let port = PORT_BASE + 101;
    let base_url = format!("http://127.0.0.1:{}", port);
    let mcp_url = format!("{}/mcp", base_url);

    // Setup HTTP server with Neovim
    let (_server_handle, _nvim_guard, connection_id) = setup_http_server_with_nvim(port).await?;

    let client = reqwest::Client::new();

    // Create three independent sessions
    let init_request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#;

    let mut sessions: Vec<String> = Vec::new();
    for i in 0..3 {
        let response = http_post(&client, &mcp_url, init_request.to_string()).await?;
        assert_eq!(response.status(), 200);

        let session_id = response
            .headers()
            .get("mcp-session-id")
            .and_then(|h| h.to_str().ok())
            .map(|s: &str| s.to_string())
            .expect("Should have session ID");

        sessions.push(session_id);
        info!("Session {}: {}", i + 1, sessions[i]);
    }

    // Send initialized notification for each session (required by MCP protocol)
    for (i, session_id) in sessions.iter().enumerate() {
        let initialized_request =
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
        let response = client
            .post(&mcp_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .body(initialized_request)
            .send()
            .await?;
        assert_eq!(
            response.status(),
            202,
            "Client {} initialized notification should return 202",
            i + 1
        );
        info!("Client {} sent initialized notification", i + 1);
    }

    // Small delay to ensure notifications are processed
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

    // Each client reads the connections resource
    let read_resource_request =
        r#"{"jsonrpc":"2.0","id":2,"method":"resources/read","params":{"uri":"nvim-connections://"}}"#.to_string();

    for (i, session_id) in sessions.iter().enumerate() {
        let response = client
            .post(&mcp_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .body(read_resource_request.clone())
            .send()
            .await?;

        assert_eq!(
            response.status(),
            200,
            "Client {} should read resources",
            i + 1
        );

        let body: String = response.text().await?;
        assert!(
            body.contains(&connection_id),
            "Client {} should see connection {} in resource",
            i + 1,
            connection_id
        );
        info!("Client {} verified connection visibility", i + 1);
    }

    // Test connection-specific tools resource for each session
    let tools_resource_request = format!(
        r#"{{"jsonrpc":"2.0","id":3,"method":"resources/read","params":{{"uri":"nvim-tools://{}"}}}}"#,
        connection_id
    );

    for (i, session_id) in sessions.iter().enumerate() {
        let response = client
            .post(&mcp_url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", session_id)
            .body(tools_resource_request.clone())
            .send()
            .await?;

        assert_eq!(
            response.status(),
            200,
            "Client {} should read tools resource",
            i + 1
        );

        let body: String = response.text().await?;
        assert!(
            body.contains(&connection_id),
            "Tools resource for client {} should contain connection_id",
            i + 1
        );
    }

    info!("Shared connection visibility test completed successfully");

    Ok(())
}

/// Test that stale socket errors don't break the shared session
#[tokio::test]
#[traced_test]
async fn test_http_stale_socket_does_not_break_shared_session()
-> Result<(), Box<dyn std::error::Error>> {
    info!("Testing HTTP stale socket error handling");

    let port = PORT_BASE + 102;
    let base_url = format!("http://127.0.0.1:{}", port);
    let mcp_url = format!("{}/mcp", base_url);

    // Setup: Create a server with auto-connect disabled
    use hyper_util::{
        rt::{TokioExecutor, TokioIo},
        server::conn::auto::Builder,
        service::TowerToHyperService,
    };
    use rmcp::transport::{
        StreamableHttpServerConfig, StreamableHttpService,
        streamable_http_server::session::local::{LocalSessionManager, SessionConfig},
    };
    use tokio::net::TcpListener;

    // Create server with manual connect mode (no pre-existing connections)
    let server = crate::NeovimMcpServer::with_connect_mode(Some("manual".to_string()));

    // Start HTTP server with configured session manager for multi-client stability
    let addr = format!("127.0.0.1:{}", port);
    let listener = TcpListener::bind(&addr).await?;

    // Configure session manager with increased channel capacity for multi-client support
    let session_config = SessionConfig {
        channel_capacity: 64,
        keep_alive: None,
    };
    let session_manager = LocalSessionManager {
        sessions: tokio::sync::RwLock::new(std::collections::HashMap::new()),
        session_config,
    };

    let server_for_handler = server.clone();
    let _server_handle = tokio::spawn(async move {
        let service = TowerToHyperService::new(StreamableHttpService::new(
            move || Ok(server_for_handler.server_for_http_session()),
            session_manager.into(),
            StreamableHttpServerConfig {
                stateful_mode: true,
                ..Default::default()
            },
        ));

        loop {
            let (stream, _) = match listener.accept().await {
                Ok(conn) => conn,
                Err(_) => break,
            };
            let io = TokioIo::new(stream);
            let service = service.clone();

            tokio::spawn(async move {
                if let Err(e) = Builder::new(TokioExecutor::default())
                    .serve_connection(io, service)
                    .await
                {
                    tracing::error!("HTTP connection error: {}", e);
                }
            });
        }
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    let client = reqwest::Client::new();

    // Client A: Create session
    let init_request = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"1.0"}}}"#.to_string();
    let response = http_post(&client, &mcp_url, init_request.clone()).await?;

    assert_eq!(response.status(), 200);
    let session_id_a = response
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s: &str| s.to_string())
        .expect("Should have session ID");

    // Client A: Send initialized notification (required by MCP protocol)
    let initialized_request =
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
    let response_init = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_a)
        .body(initialized_request)
        .send()
        .await?;
    assert_eq!(
        response_init.status(),
        202,
        "Initialized notification should return 202"
    );

    // Client A: Try to connect to a non-existent (stale) socket
    let stale_target = "/tmp/non-existent-stale-socket-test.sock";
    let connect_request = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"connect","arguments":{{"target":"{}"}}}}}}"#,
        stale_target
    );

    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_a)
        .body(connect_request)
        .send()
        .await?;

    // Should fail with a clear error about the connection
    assert_eq!(
        response.status(),
        200,
        "Request should return 200 even on error"
    );

    let body: String = response.text().await?;
    info!("Stale socket response: {}", body);

    // The error should indicate connection failure, not session/channel issues
    assert!(
        !body.contains("Channel closed"),
        "Error should not be 'Channel closed': {}",
        body
    );
    assert!(
        body.contains("error")
            || body.contains("Error")
            || body.contains("Connection")
            || body.contains("failed"),
        "Response should contain error indication: {}",
        body
    );

    // Now start a real Neovim instance and connect successfully
    let ipc_path = crate::test_utils::generate_random_ipc_path();
    let nvim_guard = crate::test_utils::setup_test_neovim_instance(&ipc_path).await?;

    // Client B: Create new session (should work even after stale socket error)
    let response = http_post(&client, &mcp_url, init_request.clone()).await?;

    assert_eq!(
        response.status(),
        200,
        "New session should work after stale socket error"
    );
    let session_id_b = response
        .headers()
        .get("mcp-session-id")
        .and_then(|h| h.to_str().ok())
        .map(|s: &str| s.to_string())
        .expect("Should have session ID for client B");

    // Client B: Send initialized notification
    let initialized_request =
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#.to_string();
    let response_b_init = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_b)
        .body(initialized_request)
        .send()
        .await?;
    assert_eq!(
        response_b_init.status(),
        202,
        "Client B initialized notification should return 202"
    );

    // Client B: Connect to valid socket
    let connect_request = format!(
        r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"connect","arguments":{{"target":"{}"}}}}}}"#,
        ipc_path
    );

    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_b)
        .body(connect_request)
        .send()
        .await?;

    let body: String = response.text().await?;
    info!("Valid socket connect response: {}", body);

    // Should succeed or contain success indication
    assert!(
        body.contains("result") || body.contains("Connected") || body.contains("success"),
        "Should be able to connect to valid socket: {}",
        body
    );

    // Client A should still work (shared state should not be corrupted)
    let tools_request = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#.to_string();
    let response = client
        .post(&mcp_url)
        .header("Content-Type", "application/json")
        .header("Accept", "application/json, text/event-stream")
        .header("mcp-session-id", &session_id_a)
        .body(tools_request)
        .send()
        .await?;

    assert_eq!(
        response.status(),
        200,
        "Client A should still work after stale socket error"
    );

    let body: String = response.text().await?;
    assert!(
        !body.contains("Channel closed"),
        "Client A should not get 'Channel closed' error: {}",
        body
    );

    info!("Stale socket error handling test completed successfully");

    // Cleanup
    drop(nvim_guard);

    Ok(())
}
