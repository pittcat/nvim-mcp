# Findings

- `client.log` 的主要失败来自 `exec_lua` 调用了错误的 Neovim Lua API，典型错误为 `nvim_buf_set_virtual_text` 参数数量不匹配。
- `debug_log.txt` 显示失败请求完成后，请求级 channel 被关闭，后续 `GET /` 或 `GET /mcp` 恢复流可能命中 `Channel closed: Some(...)` 并返回 `500 Internal Server Error`。
- `client2.log` 证明基础命令链路 `get_targets -> connect -> cursor_position -> list_buffers -> exec_lua -> navigate` 可以成功，说明核心 MCP/Rust 通道不是全局失效。
- `client2.log` 中 `connect_tcp` 对 Unix socket 路径报 `invalid socket address`，属于调用方式不匹配，预期内失败。
