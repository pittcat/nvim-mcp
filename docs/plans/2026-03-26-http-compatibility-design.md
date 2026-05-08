# HTTP Compatibility Design

**Date:** 2026-03-26

**Goal:** Make `nvim-mcp`'s HTTP transport compatible with Claude Code's MCP HTTP probing so the server is no longer surfaced as `needs authentication`.

## Problem

`nvim-mcp` is currently exposed to Claude Code as an HTTP MCP server via `http://127.0.0.1:8080`. The server itself is healthy and completes normal MCP initialization, but Claude Code marks it as `needs authentication`.

Observed behavior:

- Plain `GET` requests with `Accept: text/event-stream` receive `401 Unauthorized: Session ID is required`.
- Normal `POST initialize` requests succeed and return a valid `mcp-session-id`.
- Other Claude-connected HTTP MCP servers on this machine return `200 OK` for the initial SSE probe rather than `401`.

This strongly suggests Claude's initial connectivity check interprets the `401` as an authentication requirement rather than a stream/session requirement.

## Constraints

- Preserve the existing stateful HTTP session behavior.
- Avoid changes to tool routing, Neovim connection management, or session semantics after initialization.
- Keep the fix localized to the HTTP entrypoint.
- Maintain compatibility with existing integration tests around multi-session HTTP behavior.

## Approaches Considered

### Option A: HTTP compatibility shim at the transport boundary

Wrap the existing `StreamableHttpService` with a thin Hyper service that:

- returns `200 OK` with a minimal SSE keep-alive response for `GET` requests that advertise `text/event-stream` and do not yet provide `mcp-session-id`
- downgrades missing-session `401` responses from the underlying transport to `400 Bad Request`

Pros:

- Smallest change with the widest client compatibility benefit
- Leaves current session model intact
- Easy to verify with focused HTTP integration tests

Cons:

- Adds a compatibility layer around the transport

### Option B: Only remap `401` to `400`

Pros:

- Very small code change

Cons:

- Might still fail if Claude expects the initial SSE probe itself to succeed with `200`

### Option C: Rework HTTP mode to be stateless

Pros:

- Might align better with some clients

Cons:

- Much larger behavioral change
- Risks breaking current multi-session HTTP flow

## Chosen Design

Use Option A.

Implement a small HTTP compatibility shim around the existing streamable HTTP service:

1. Intercept initial SSE probe requests:
   - Method: `GET`
   - `Accept` includes `text/event-stream`
   - No `mcp-session-id` header
   - Respond with `200 OK`, `content-type: text/event-stream`, and a minimal keep-alive body

2. Forward all other requests to the existing `StreamableHttpService`.

3. If the underlying service returns `401 Unauthorized` and the response body indicates a missing session ID, rewrite that response to `400 Bad Request`.

This preserves the current initialize flow while avoiding the authentication-shaped error that Claude is currently misclassifying.

## Testing Strategy

Add focused integration coverage that proves:

- initial SSE probe without session ID returns `200 OK`
- a malformed follow-up request without session ID returns `400 Bad Request` instead of `401`
- normal initialize flow still succeeds and returns `mcp-session-id`

Then run the existing HTTP integration suite and the full repository test command.
