# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the Rust code. `src/server/` implements the MCP server, tool routing, resources, and transport logic; `src/neovim/` contains the Neovim client, connection handling, embedded Lua helpers, and integration tests. `src/testdata/` holds Go, Lua, Zig, and TypeScript fixtures used by LSP-heavy tests. The Neovim plugin entrypoint lives in `lua/nvim-mcp/init.lua`. User and developer documentation is under `docs/`, and helper scripts live in `scripts/`.

## Build, Test, and Development Commands
Use `nix develop .` for the pinned toolchain and test dependencies.

- `cargo build` builds the binary locally.
- `cargo run -- --connect auto` runs the server against the current project’s Neovim socket.
- `./scripts/run-test.sh -- --show-output` builds `nvim-mcp` and runs the full Rust test suite.
- `./scripts/run-test.sh -- --skip=integration_tests --show-output` runs fast checks when Neovim is unavailable.
- `./scripts/run-cov.sh -- --show-output` generates LLVM coverage output in `target/coverage/result/`.
- `pre-commit run --all-files` runs the same formatting, lint, and Markdown checks enforced in CI.

## Coding Style & Naming Conventions
Follow Rust 2024 idioms and keep `cargo fmt`/`rustfmt` output authoritative. The repo enforces `clippy --all-targets --all-features -D warnings`, so treat warnings as failures. Use `snake_case` for functions, modules, and test names; `CamelCase` for types; and keep tool parameter structs descriptive, for example `ReadDocumentRequest`. Lua files are formatted with StyLua using 4-space indentation and a 120-column width.

## Testing Guidelines
Integration coverage lives mainly in `src/server/integration_tests.rs` and `src/neovim/integration_tests.rs`. Prefer focused unit tests beside the code for pure logic, and add integration tests when changing MCP tools, Neovim RPC behavior, or connection management. Name tests after the observed behavior, such as `test_tcp_connection_lifecycle`.

## Commit & Pull Request Guidelines
Recent history favors short, imperative subjects; dependency updates use `Bump <crate> from <old> to <new> (#123)`. Keep commits scoped and descriptive. PRs should explain the user-visible change, mention any affected tools/resources/docs, and note verification commands run. Include screenshots only when changing rendered documentation or UI-facing Neovim behavior.
