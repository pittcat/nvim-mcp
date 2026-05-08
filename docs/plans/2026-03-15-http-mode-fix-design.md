# HTTP Mode Fix Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `nvim-mcp --http-port ... --connect auto` preserve its preconnected Neovim state across HTTP sessions and stop the local example config from implying a broken setup.

**Architecture:** Keep the current startup flow that creates one `NeovimMcpServer`, auto-connects it, and discovers Lua tools before entering the transport loop. Change the HTTP transport factory so it reuses that same server state instead of constructing fresh `NeovimMcpServer` values for each session. Add a focused regression test around the extracted HTTP factory helper so the state-sharing expectation is explicit and stable.

**Tech Stack:** Rust 2024, `tokio`, `rmcp`, `dashmap`, repo unit tests, docs JSON example.

---

## Approach Options

1. Clone and reuse one shared `NeovimMcpServer`
   - Pros: minimal change, preserves current architecture, easy to test.
   - Cons: requires `NeovimMcpServer` and `HybridToolRouter` to implement safe `Clone`.

2. Move HTTP startup to an `Arc<NeovimMcpServer>`-backed wrapper type
   - Pros: explicit shared ownership.
   - Cons: larger refactor, touches more signatures, unnecessary if plain clone can share interior `Arc` state.

3. Re-run auto-connect inside each HTTP session factory
   - Pros: avoids clone work.
   - Cons: wrong behavior, repeated side effects, slower, can create duplicate connections.

## Recommendation

Use option 1. The server already stores connection and dynamic-tool state behind `Arc<DashMap<...>>`, so implementing clone-based reuse is the smallest coherent fix.

## Test Strategy

- Add a unit test that builds an auto-connected-like server state in memory by inserting a marker into `nvim_clients` and then verifies the HTTP factory returns a server with the same visible state.
- Run the new focused test first and confirm it fails against the old implementation.
- After the fix, re-run the focused test and one broader relevant Rust test target.

## Docs Strategy

- Keep `docs/nvim.json` as a local development example file, but add an inline note clarifying that the `nvim-mcp-http` entry only starts an HTTP server process.
- State that an actual HTTP MCP client must connect by URL to that server instead of treating this entry as a ready-made `stdio` MCP config.
