# MCP Resources

Access connection and buffer information through structured URI schemes:

## Available Resources

### Connection Monitoring

- **`nvim-connections://`**: List all active Neovim connections
  - Returns array of connection objects with `id` and `target` information
  - Useful for monitoring multiple concurrent Neovim instances

### Tool Registration Overview

- **`nvim-tools://`**: Overview of all tools and their connection mappings
  - Shows static tools (available to all connections) and dynamic tools
    (connection-specific)
  - Useful for understanding tool availability across connections

- **`nvim-tools://{connection_id}`**: List of tools available for a specific connection
  - Includes both static and connection-specific dynamic tools
  - Provides detailed view of tools available for a particular Neovim instance

## Usage Examples

### List Active Connections

```json
{
  "method": "resources/read",
  "params": {
    "uri": "nvim-connections://"
  }
}
```

### Get Tools Overview

```json
{
  "method": "resources/read",
  "params": {
    "uri": "nvim-tools://"
  }
}
```

Connection IDs are deterministic BLAKE3 hashes of the target string for
consistent identification across sessions.
