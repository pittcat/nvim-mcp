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
nvim-mcp --http-port 8080 --http-host 0.0.0.0
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

### Starting HTTP Server Mode

```bash
# Start HTTP server on default localhost:8080
nvim-mcp --http-port 8080

# Bind to all interfaces
nvim-mcp --http-port 8080 --http-host 0.0.0.0

# With custom logging
nvim-mcp --http-port 8080 --log-file ./nvim-mcp.log --log-level debug
```

Important: `--http-port` only changes how `nvim-mcp` serves MCP after it starts. If your MCP client is still using a `command`/`args` child-process configuration, that client is still expecting `stdio` unless it has a separate HTTP URL transport configuration. Do not assume that adding `--http-port` to a `stdio` server entry automatically converts the client side to HTTP.

For Claude Code specifically:

```bash
# Stdio mode: Claude Code starts nvim-mcp as a subprocess
claude mcp add -s project nvim-mcp-stdio -- /Users/pittcat/Dev/Rust/nvim-mcp/target/release/nvim-mcp \
  --connect auto --log-file ./nvim-mcp.log --log-level debug

# HTTP mode: start nvim-mcp separately, then point Claude Code at the URL
nvim-mcp --http-port 8080 --connect auto --log-file ./nvim-mcp-http.log --log-level debug
claude mcp add -s project --transport http nvim-mcp-http http://127.0.0.1:8080
```

The `nvim-mcp-http` entry in `docs/nvim.json` does not start the HTTP server for you. Start the `nvim-mcp --http-port 8080 ...` process yourself first, then let Claude Code connect to that URL.
