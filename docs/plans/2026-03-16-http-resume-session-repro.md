# HTTP Resume Session Repro Script Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a Python script that reproduces the `Internal server error when resume session: Session error: Channel closed` issue against a running `nvim-mcp` HTTP server and confirms it from the debug log.

**Architecture:** Keep the script standalone and dependency-free by using Python's standard library HTTP client and filesystem APIs. Drive a minimal MCP flow over Streamable HTTP, force one `exec_lua` failure inside a valid session, then issue follow-up requests and scan the appended log region for the `resume session` error.

**Tech Stack:** Python 3 standard library, existing `nvim-mcp` HTTP server, `target/debug/debug_log.txt`.

---

### Task 1: Add a failing Python unit test

**Files:**
- Create: `scripts/tests/test_repro_http_resume_session_bug.py`
- Test: `scripts/tests/test_repro_http_resume_session_bug.py`

**Step 1: Write the failing test**

Add a unit test that expects a helper to detect `resume session` / `Channel closed` lines from a log snippet and parse simple SSE `data:` events.

**Step 2: Run test to verify it fails**

Run: `python3 -m unittest scripts.tests.test_repro_http_resume_session_bug -v`
Expected: FAIL because the reproduction module does not exist yet.

**Step 3: Write minimal implementation**

Create a Python module under `scripts/` with just enough helpers for the tests to import.

**Step 4: Run test to verify it passes**

Run: `python3 -m unittest scripts.tests.test_repro_http_resume_session_bug -v`
Expected: PASS.

### Task 2: Implement the HTTP reproduction flow

**Files:**
- Create: `scripts/repro_http_resume_session_bug.py`

**Step 1: Add MCP HTTP helpers**

Implement helpers for:
- `initialize`
- `notifications/initialized`
- `tools/call`
- parsing SSE `data:` events into JSON-RPC payloads

**Step 2: Add target selection**

Support `--target <socket>` and `--target auto`, where `auto` tries `/tmp/nvim-mcp.*.sock` candidates until `connect` succeeds.

**Step 3: Add bug trigger flow**

Implement:
- capture current log size
- initialize a session
- connect to Neovim
- call `exec_lua` with a failing Lua snippet
- send one or more follow-up requests in the same session
- sleep briefly and scan the appended log segment

**Step 4: Add CLI and exit behavior**

Exit `0` only when the script sees the target `resume session` error in the log. Exit non-zero with a concise diagnostic when the issue is not reproduced.

### Task 3: Verify against the current environment

**Files:**
- Create: `scripts/repro_http_resume_session_bug.py`
- Create: `scripts/tests/test_repro_http_resume_session_bug.py`

**Step 1: Run focused unit tests**

Run: `python3 -m unittest scripts.tests.test_repro_http_resume_session_bug -v`

**Step 2: Run the reproduction script against the local server**

Run: `python3 scripts/repro_http_resume_session_bug.py --url http://127.0.0.1:8080/mcp --target auto --log-path target/debug/debug_log.txt`

**Step 3: Confirm fresh evidence**

Verify whether the script output and appended log lines contain `resume session` and `Channel closed`. If not, report the exact observed behavior instead of claiming reproduction.
