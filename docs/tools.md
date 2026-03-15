# MCP Tools Reference

The server provides 33 MCP tools for interacting with Neovim:

## Connection Management

- **`get_targets`**: Discover available Neovim targets
  - Returns list of discoverable Neovim socket paths created by the plugin
  - No parameters required

- **`connect`**: Connect via Unix socket/named pipe
  - Parameters: `target` (string) - Socket path from get_targets
  - Returns: `connection_id` (string) - Deterministic connection identifier

- **`connect_tcp`**: Connect via TCP
  - Parameters: `target` (string) - TCP address (e.g., "127.0.0.1:6666")
  - Returns: `connection_id` (string) - Deterministic connection identifier

- **`disconnect`**: Disconnect from specific Neovim instance
  - Parameters: `connection_id` (string) - Connection identifier to disconnect

## Connection-Aware Tools

All tools below require a `connection_id` parameter from the connection
establishment phase:

### Navigation and Positioning

- **`navigate`**: Navigate to a specific position in the current buffer or open
  a file at a specific position
  - Parameters: `connection_id` (string), `document` (DocumentIdentifier),
    `line` (number), `character` (number) (all positions are 0-indexed)
  - Returns: Navigation result with `path` (string), `line` (number, 0-based),
    `column` (number, 0-based)

- **`cursor_position`**: Get the current cursor position: buffer name,
  and zero-based row/col index
  - Parameters: `connection_id` (string) - Target Neovim connection

### Buffer Operations

- **`list_buffers`**: List all open buffers with names and line counts
  - Parameters: `connection_id` (string) - Target Neovim connection

- **`read`**: Read document content with universal document identification
  - Parameters: `connection_id` (string), `document` (DocumentIdentifier),
    `start` (number, optional, default: 0) - Start line index (0-based),
    `end` (number, optional, default: -1) - End line index, exclusive
    (0-based, -1 for end of buffer)
  - Returns: Document content as text
  - Notes: Supports reading from buffer IDs, project-relative paths, and
    absolute file paths with optional line range specification

- **`buffer_diagnostics`**: Get diagnostics for a specific buffer
  - Parameters: `connection_id` (string), `id` (number) - Buffer ID

## Code Execution

- **`exec_lua`**: Execute Lua code in Neovim
  - Parameters: `connection_id` (string), `code` (string) - Lua code to execute
