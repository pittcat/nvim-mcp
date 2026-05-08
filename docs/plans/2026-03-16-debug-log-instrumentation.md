# Debug Log Instrumentation Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `nvim-mcp` overwrite and recreate root-level `debug_log.txt` on each run, and record structured server + embedded Neovim client debug logs with enough context to diagnose session, connect, Lua execution, and request/response failures.

**Architecture:** Keep the existing `tracing` stack, but replace the current plain file logging path with a dedicated formatter/writer for `debug_log.txt`. Add consistent context extraction helpers so tool requests, session setup, connection lookup, Lua execution, and failure paths emit the same request/session/connection fields.

**Tech Stack:** Rust 2024, `tracing`, `tracing-subscriber`, `tracing-appender`, existing MCP server code in `src/main.rs`, `src/server/*`, and `src/neovim/client.rs`.

---

### Task 1: Add a failing logging bootstrap test

**Files:**
- Modify: `src/main.rs`
- Test: `src/main.rs`

**Step 1: Write the failing test**

Add a small unit test around a new helper that prepares `debug_log.txt`, asserting an existing file is removed and recreated empty.

**Step 2: Run test to verify it fails**

Run: `cargo test prepare_debug_log_file -- --nocapture`
Expected: FAIL because helper does not exist yet.

**Step 3: Write minimal implementation**

Create a helper in `src/main.rs` that deletes an existing log file and creates a new empty UTF-8 file.

**Step 4: Run test to verify it passes**

Run: `cargo test prepare_debug_log_file -- --nocapture`
Expected: PASS.

### Task 2: Route startup logs to root `debug_log.txt`

**Files:**
- Modify: `src/main.rs`

**Step 1: Replace ad-hoc `--log-file` behavior for the default path**

Make startup initialize logging to `<cwd>/debug_log.txt` when no explicit file path is provided.

**Step 2: Add environment/version/config startup logs**

Emit `[START]` and environment/version/config lines at startup.

**Step 3: Add end-of-run logging**

Log completion/failure with elapsed time before process exit.

### Task 3: Add request/session context helpers

**Files:**
- Modify: `src/server/core.rs`
- Modify: `src/server/tools.rs`
- Modify: `src/server/resources.rs`

**Step 1: Introduce helper(s) to extract context IDs**

Capture request id, progress token, tool use id, and session-adjacent values from `RequestContext`.

**Step 2: Add structured logs to connection lookup and tool entry points**

Log tool name, connection id, key arguments, and data flow summaries at entry/exit/error.

**Step 3: Add explicit logs around Lua tool discovery/setup**

Record connection id, target, discovered tool count, and failure causes.

### Task 4: Add embedded Neovim client instrumentation

**Files:**
- Modify: `src/neovim/client.rs`

**Step 1: Add connection-level logs**

Record target, connection status, and connection failures.

**Step 2: Add Lua execution logs**

Log code size, first-line preview, success/failure, and error stack snippets.

**Step 3: Add data-flow logs on read/navigate helpers**

Record document identifiers, ranges, and returned sizes.

### Task 5: Verify behavior

**Files:**
- Modify: `src/main.rs`
- Modify: `src/server/core.rs`
- Modify: `src/server/tools.rs`
- Modify: `src/server/resources.rs`
- Modify: `src/neovim/client.rs`

**Step 1: Run focused unit tests**

Run: `cargo test prepare_debug_log_file -- --nocapture`

**Step 2: Run targeted Rust tests/build**

Run: `cargo test --lib`
Run: `cargo build`

**Step 3: Sanity-check generated log file contract**

Verify `debug_log.txt` is recreated cleanly and contains start/environment lines in the expected format.
