# HTTP Compatibility Shim Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Make `nvim-mcp`'s HTTP mode compatible with Claude Code's initial MCP HTTP probing behavior.

**Architecture:** Add a thin Hyper-level compatibility wrapper around the existing `StreamableHttpService`. The wrapper will special-case the initial SSE probe and normalize missing-session unauthorized responses without changing the current stateful MCP session model.

**Tech Stack:** Rust 2024, Hyper, hyper-util, rmcp streamable HTTP transport, reqwest integration tests.

---

### Task 1: Add failing regression coverage for the probe path

**Files:**
- Modify: `src/server/integration_tests.rs`

**Step 1: Write the failing test**

Add integration coverage that starts the HTTP server and issues `GET` with `Accept: text/event-stream` and no `mcp-session-id`, asserting the response is `200 OK` and `content-type: text/event-stream`.

**Step 2: Run test to verify it fails**

Run: `cargo test http_initial_sse_probe_without_session_id_returns_ok_stream -- --show-output`

Expected: FAIL because the current transport returns `401 Unauthorized`.

**Step 3: Write minimal implementation**

Add the HTTP compatibility shim in the HTTP server setup path.

**Step 4: Run test to verify it passes**

Run: `cargo test http_initial_sse_probe_without_session_id_returns_ok_stream -- --show-output`

Expected: PASS

### Task 2: Add failing regression coverage for unauthorized session handling

**Files:**
- Modify: `src/server/integration_tests.rs`

**Step 1: Write the failing test**

Add a test that sends a follow-up JSON-RPC request without `mcp-session-id` and asserts the status becomes `400 Bad Request`.

**Step 2: Run test to verify it fails**

Run: `cargo test http_missing_session_request_returns_bad_request_not_unauthorized -- --show-output`

Expected: FAIL because the current transport returns `401 Unauthorized`.

**Step 3: Write minimal implementation**

Map missing-session unauthorized responses from the underlying transport to `400 Bad Request`.

**Step 4: Run test to verify it passes**

Run: `cargo test http_missing_session_request_returns_bad_request_not_unauthorized -- --show-output`

Expected: PASS

### Task 3: Verify no regressions in normal HTTP MCP flow

**Files:**
- Modify: `src/main.rs`
- Test: `src/server/integration_tests.rs`

**Step 1: Run targeted HTTP integration tests**

Run: `cargo test http_ -- --show-output`

Expected: PASS for the new probe coverage and the existing HTTP session tests.

**Step 2: Run full repository verification**

Run: `./scripts/run-test.sh -- --show-output`

Expected: PASS
