# Memory Benchmark Design

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Add a repeatable memory benchmark script for `nvim-mcp` and update the memory optimization report with measured RSS data and rerun instructions.

**Architecture:** Implement a dependency-free Python script under `scripts/` that builds or uses the existing debug binary, launches controlled benchmark scenarios, samples process RSS via `ps`, and writes timestamped JSON results to `target/benchmarks/memory/`. Keep the script logic decomposed into pure helpers so it can be tested with `unittest` without requiring real Neovim processes for every test. Update the Chinese report with the measured numbers from a real run and document how to rerun the benchmark.

**Tech Stack:** Python 3 standard library, macOS/Linux `ps`, existing Rust binary, headless Neovim, Markdown docs.

---

## Scope Update

- Promote `release` measurements to the primary data source in the report.
- Add a `live snapshot` mode that measures currently running `target/release/nvim-mcp` processes instead of only synthetic benchmark processes.
- Demote old `debug` measurements so they are no longer presented as the main conclusion.

### Task 1: Add failing tests for the benchmark helpers

**Files:**
- Create: `scripts/test_bench_memory.py`
- Create: `scripts/bench_memory.py`

**Step 1: Write the failing test**

Add `unittest` coverage for:
- parsing RSS from `ps` output
- computing MiB from KiB
- building the benchmark JSON structure
- choosing the default output directory under `target/benchmarks/memory/`

**Step 2: Run test to verify it fails**

Run: `python3 -m unittest scripts.test_bench_memory -v`
Expected: FAIL because `scripts.bench_memory` does not exist yet.

**Step 3: Write minimal implementation**

Create `scripts/bench_memory.py` with helper functions and stubs needed by the tests.

**Step 4: Run test to verify it passes**

Run: `python3 -m unittest scripts.test_bench_memory -v`
Expected: PASS

### Task 2: Implement real benchmark scenarios

**Files:**
- Modify: `scripts/bench_memory.py`
- Test: `scripts/test_bench_memory.py`

**Step 1: Write the failing test**

Add tests for argument parsing and JSON result writing with deterministic timestamps/output paths.

**Step 2: Run test to verify it fails**

Run: `python3 -m unittest scripts.test_bench_memory -v`
Expected: FAIL on missing CLI behavior or file writing helpers.

**Step 3: Write minimal implementation**

Implement:
- idle `nvim-mcp` scenario
- one connected headless Neovim scenario
- multi-process same-Neovim scenario
- JSON persistence with a timestamped filename

**Step 4: Run test to verify it passes**

Run: `python3 -m unittest scripts.test_bench_memory -v`
Expected: PASS

### Task 3: Run the benchmark and update the report

**Files:**
- Modify: `docs/mcp-memory-optimization-report-zh.md`
- Modify: `progress.md`
- Modify: `findings.md`

**Step 1: Run the benchmark script**

Run: `python3 scripts/bench_memory.py`
Expected: JSON result written under `target/benchmarks/memory/`

**Step 2: Update the report**

Add:
- measured RSS numbers
- interpretation for the user’s multi-Claude-Code scenario
- rerun command and output location

**Step 3: Verify the report references the benchmark output format**

Run: `rg -n "bench_memory.py|target/benchmarks/memory|RSS" docs/mcp-memory-optimization-report-zh.md`
Expected: matches for the new measured-results section

### Task 4: Full verification

**Files:**
- Verify: `scripts/bench_memory.py`
- Verify: `scripts/test_bench_memory.py`
- Verify: `docs/mcp-memory-optimization-report-zh.md`

**Step 1: Run unit tests**

Run: `python3 -m unittest scripts.test_bench_memory -v`
Expected: PASS

**Step 2: Run the benchmark end-to-end**

Run: `python3 scripts/bench_memory.py`
Expected: PASS with JSON output path printed

**Step 3: Optional compile check**

Run: `cargo build --quiet`
Expected: PASS
