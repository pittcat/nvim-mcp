# Usage Guide

This guide covers detailed usage patterns, workflows, and transport modes for the
nvim-mcp server.

## Quick Start

### 1. Setup Neovim Integration

#### Option A: Using Neovim Plugin (Recommended)

With a plugin manager like `lazy.nvim`:

```lua
return {
    "linw1995/nvim-mcp",
    -- install the mcp server binary automatically
    -- build = "cargo install --path .",
    build = [[
      nix build .#nvim-mcp
      nix profile remove nvim-mcp
      nix profile install .#nvim-mcp
    ]],
    opts = {},
}
```

This plugin automatically creates a Unix-Socket/pipe for MCP connections.

#### Option B: Manual Setup

Start Neovim with TCP listening or creating Unix-Socket:

```bash
nvim --listen 127.0.0.1:6666

# Or creating Unix-Socket
nvim --listen ./nvim.sock
```

Or add to your Neovim config:

```lua
vim.fn.serverstart("127.0.0.1:6666")

-- Or creating Unix-Socket
vim.fn.serverstart("./nvim.sock")
```

#### Option C: Using Install Script (HTTP Daemon Mode)

For a persistent HTTP server that runs as a system service:

```bash
# Build from source and install as a system service
./install.sh

# Use an existing binary
./install.sh --binary ./target/release/nvim-mcp

# Custom HTTP port
./install.sh --port 9090

# Binary already in ~/.local/bin/, configure service only
./install.sh --skip-build
```

The install script will:
- Build the binary from source (or use provided binary)
- Install to `~/.local/bin/`
- Configure a system service (systemd on Linux, launchd on macOS)
- Start the HTTP server with auto-connect mode

After installation, configure your MCP client:

```bash
claude mcp add -s local nvim --transport http http://127.0.0.1:8080
```

### 2. Start the Server working with various clients

```bash
# Configure claude to auto-connect to current project Neovim instances (recommended)
claude mcp add -s local nvim -- nvim-mcp --log-file . \
  --log-level debug --connect auto

# Your full options to start the server
# Start as stdio MCP server (default, manual connection mode)
nvim-mcp

# Auto-connect to current project Neovim instances
nvim-mcp --connect auto

# Connect to specific target (TCP address or socket path)
nvim-mcp --connect 127.0.0.1:6666
nvim-mcp --connect /tmp/nvim.sock

# With custom logging
nvim-mcp --log-file ./nvim-mcp.log --log-level debug

# HTTP server mode with auto-connection
nvim-mcp --http-port 8080 --connect auto

# HTTP server mode with custom bind address
nvim-mcp --http-port 8080 --http-host 127.0.0.1
```

## Command Line Options

- `--connect <MODE>`: Connection mode (default: manual)
  - `manual`: Traditional workflow using get_targets and connect tools
  - `auto`: Automatically connect to all project-associated Neovim instances
  - Specific target: TCP address (e.g., `127.0.0.1:6666`) or absolute socket path
- `--log-file <PATH>`: Path to log file (defaults to stderr)
- `--log-level <LEVEL>`: Log level (trace, debug, info, warn, error;
  defaults to info)
- `--http-port <PORT>`: Enable HTTP server mode on the specified port
- `--http-host <HOST>`: HTTP server bind address (defaults to 127.0.0.1)

## Usage Workflows

Once both the MCP server and Neovim are running, here are the available workflows:

### Automatic Connection Mode (Recommended)

When using `--connect auto`, the server automatically discovers and connects to
Neovim instances associated with your current project:

1. **Start server with auto-connect**:

   ```bash
   nvim-mcp --connect auto
   ```

2. **Server automatically**:
   - Detects current project root (git repository or working directory)
   - Finds all Neovim instances for the current project
   - Establishes connections with deterministic `connection_id`s
   - Reports connection status and IDs
3. **Use connection-aware tools directly**:
   - Server logs will show the `connection_id`s for connected instances
   - Use tools like `list_buffers`, `read`, etc. with these IDs
   - Access resources immediately without manual connection setup

### Specific Target Mode

For direct connection to a known target:

1. **Connect to specific target**:

   ```bash
   # TCP connection
   nvim-mcp --connect 127.0.0.1:6666

   # Unix socket connection
   nvim-mcp --connect /tmp/nvim.sock
   ```

2. **Server automatically connects and reports the `connection_id`**
3. **Use connection-aware tools with the reported ID**

### Manual Connection Mode (Traditional)

For traditional discovery-based workflow:

1. **Discover available Neovim instances**:
   - Use `get_targets` tool to list available socket paths
2. **Connect to Neovim**:
   - Use `connect` tool with a socket path from step 1
   - Save the returned `connection_id` for subsequent operations
3. **Perform operations**:
   - Use tools like `list_buffers`, `read`, etc. with your `connection_id`
   - Access resources like `nvim-connections://` or `nvim-tools://`
4. **Optional cleanup**:
   - Use `disconnect` tool when completely done

## HTTP Server Transport

The server supports HTTP transport mode for web-based integrations and
applications that cannot use stdio transport. This is useful for web
applications, browser extensions, or other HTTP-based MCP clients.

> **Tip**: Use the [`install.sh`](#option-c-using-install-script-http-daemon-mode) script to automatically set up HTTP server as a system service.

### Starting HTTP Server Mode

```bash
# Start HTTP server on default localhost:8080
nvim-mcp --http-port 8080

# Bind to all interfaces
nvim-mcp --http-port 8080 --http-host 127.0.0.1

# With custom logging
nvim-mcp --http-port 8080 --log-file ./nvim-mcp.log --log-level debug
```

Important: `--http-port` only changes how `nvim-mcp` serves MCP after it starts. If your MCP client is still using a `command`/`args` child-process configuration, that client is still expecting `stdio` unless it has a separate HTTP URL transport configuration. Do not assume that adding `--http-port` to a `stdio` server entry automatically converts the client side to HTTP.

For Claude Code specifically:

```bash
# Stdio mode: Claude Code starts nvim-mcp as a subprocess
claude mcp add -s project nvim-mcp-stdio -- /Users/pittcat/Dev/Rust/nvim-mcp/target/release/nvim-mcp \
  --connect auto --log-file ./nvim-mcp.log --log-level debug

# HTTP mode: start nvim-mcp separately, then point Claude Code at the MCP endpoint
./target/release/nvim-mcp --http-port 8080 --http-host 127.0.0.1 --connect auto

# With explicit log file and debug logging
nvim-mcp --http-port 8080 --http-host 127.0.0.1 --connect auto --log-file ./nvim-mcp-http.log --log-level debug
claude mcp add -s project --transport http nvim-mcp-http http://127.0.0.1:8080
```

The `nvim-mcp-http` entry in `docs/nvim.json` does not start the HTTP server for you. Start the `nvim-mcp --http-port 8080 ...` process yourself first, then let Claude Code connect to `http://127.0.0.1:8080`.

For local Claude Code usage, prefer `--http-host 127.0.0.1` over `0.0.0.0`. On macOS, a different process bound to a more specific address such as `127.0.0.1:8080` can intercept localhost traffic even if `nvim-mcp` is also listening on `0.0.0.0:8080`.

### Multi-Client HTTP Mode (Shared Daemon)

The HTTP server supports multiple concurrent MCP clients sharing the same `nvim-mcp` daemon process. This enables scenarios like multiple Claude Code windows or other MCP clients connecting to a single long-running server.

#### Usage Pattern

> **Note**: You can use `install.sh` to automatically set up the HTTP server as a persistent system service. See [Option C](#option-c-using-install-script-http-daemon-mode) above.

```bash
# 1. Start the HTTP server (single daemon)
nvim-mcp --http-port 8080 --connect auto --log-file ./nvim-mcp.log --log-level info

# 2. Configure multiple clients to connect to the same MCP endpoint
# Client A (e.g., first Claude Code window)
claude mcp add -s project nvim-shared --transport http http://127.0.0.1:8080

# Client B (e.g., second Claude Code window)
claude mcp add -s project nvim-shared --transport http http://127.0.0.1:8080
```

#### Shared Connection Semantics

When using multi-client HTTP mode, the following semantics apply:

1. **Connection Visibility**: Neovim connections created by one client are visible to all clients sharing the same daemon. All clients can see and use the same `connection_id`s.

2. **Connection Persistence**: Connections persist across client sessions. If Client A creates a connection, Client B can use the same `connection_id` without reconnecting.

3. **Concurrent Requests**: Multiple clients can make requests concurrently without interfering with each other, even when accessing the same Neovim instance.

4. **Session Independence**: Each client maintains its own MCP session while sharing the underlying Neovim connection pool.

#### Configuration Tuning

The HTTP server is configured with settings optimized for multi-client scenarios:

- **Channel Capacity**: 64 concurrent requests (increased from default 16)
- **Keep-Alive**: Sessions remain active until explicitly closed (no timeout)

These settings ensure stable operation when multiple clients are active simultaneously.

#### Session Recovery

In multi-client HTTP mode, sessions may occasionally need to be recovered:

**Session Resume Behavior:**

- When resuming a session with an invalid or expired `mcp-session-id`, the
  server returns a 401 Unauthorized response instead of HTTP 500
- This is a recoverable error - clients should re-initialize their MCP session
- The error response includes clear messaging to guide recovery

**Recovery Steps:**

1. If you receive a session resume error, your client should re-initialize:
   - Re-send the `initialize` request to establish a new session
   - Re-discover and reconnect to Neovim instances as needed
2. Existing Neovim connections remain intact during session recovery
3. Other clients sharing the daemon are not affected

**Preventing Session Issues:**

- Use the server's default multi-client configuration (64 channel capacity,
  no keep-alive timeout)
- Avoid restarting the HTTP server while clients are active
- Monitor `debug_log.txt` for session-related warnings

#### Limitations and Best Practices

**Limitations:**

1. **Socket Lifecycle**: If a Neovim socket becomes stale (Neovim process terminates), the connection will fail gracefully with a clear error message, but other clients/sessions are not affected.

2. **No Built-in Authentication**: The HTTP server binds to localhost by default. Use `--http-host` with caution - only bind to external interfaces if you have appropriate network security in place.

3. **Client Disconnection**: When a client disconnects, other clients continue to function normally. However, consider using `--connect auto` so new clients automatically benefit from existing connections.

**Best Practices:**

1. **Use `--connect auto`**: When running as a shared daemon, auto-connection mode ensures all clients see the same pre-connected Neovim instances.

2. **Monitor Logs**: Enable appropriate logging to diagnose multi-client issues:
   ```bash
   nvim-mcp --http-port 8080 --connect auto --log-level debug
   ```

3. **Graceful Shutdown**: To stop the shared daemon, press `Ctrl+C` or send a termination signal. All sessions will be cleaned up properly.

4. **Connection Cleanup**: When done with a Neovim instance, use the `disconnect` tool to close the connection cleanly. This frees resources for all clients.

#### Troubleshooting

| Issue | Possible Cause | Solution |
|-------|---------------|----------|
| `Channel closed` errors | Session timeout or server overload | Ensure server uses default multi-client config; check logs for resource exhaustion |
| Connection not visible to other clients | Used `--connect manual` and client-specific connections | Use `--connect auto` for shared connections |
| Stale socket errors | Neovim process terminated | Restart Neovim and reconnect; stale socket errors are now handled gracefully |
| Port already in use | Another instance running | Kill existing process or use different port with `--http-port` |
| HTTP 401 Unauthorized on resume | Session expired or server restarted | Re-initialize MCP session; connections will need to be re-established |
| `connect_tcp` rejects path | Unix socket path passed to `connect_tcp` | Use `connect` tool for Unix sockets, `connect_tcp` for TCP addresses only |
