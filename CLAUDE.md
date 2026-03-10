# Flowspec

Static code analyzer that traces the flow of all data in a codebase, optimized for efficient use by AI agents during building, debugging, and within CI workflows.

## Source of Truth

The specification corpus at `.flowspec/spec/` is the **sole source of truth** for what Flowspec is, what it does, and how it does it. If you're working on Flowspec, read the spec files first:

- `intent.yaml` — WHY Flowspec exists, who it serves, trade-offs
- `architecture.yaml` — WHAT the system looks like, components, data flow
- `manifest-schema.yaml` — THE OUTPUT format, every section and field
- `diagnostics.yaml` — THE VALUE, every diagnostic pattern with detection logic
- `constraints.yaml` — BOUNDARIES, musts/must-nots/preferences
- `quality.yaml` — THE BAR, testing strategy, coverage, dogfooding
- `cli.yaml` — THE INTERFACE, every command and flag
- `conventions.yaml` — HOW code is written, patterns, naming
- `integration.yaml` — THE ECOSYSTEM, Mozart/CI/MCP connections

Decision rationale is in `.flowspec/state/decisions.log`.

## Key Constraints

- **Rust, stable toolchain.** Cargo workspace: `flowspec` (library) + `flowspec-cli` (binary).
- **Tree-sitter only.** No LSP servers. Custom semantic resolution on top of tree-sitter AST.
- **Three languages in v1:** Python, JavaScript/TypeScript, Rust.
- **AGPL-3.0 + Commercial dual license.** All deps must be AGPL-compatible.
- **Async with tokio.** Async for I/O, spawn_blocking for CPU-bound tree-sitter parsing.
- **thiserror for library errors, anyhow only in main.rs.**
- **tracing for logging, never println!/eprintln!**
- **All public functions documented with `///`.**
- **89% test coverage.** Adversarial testing. Broken tests never deferred.

## What NOT to Do

- Do NOT spawn external processes during analysis (no LSP servers)
- Do NOT use nightly Rust features
- Do NOT produce manifests larger than 10x source code size
- Do NOT analyze generated code (target/, node_modules/, __pycache__/)
- Do NOT suppress diagnostic findings to reduce false positives — use confidence levels
- Do NOT defer broken tests or ignore issues — fix or file a GitHub issue
- Do NOT use unsafe unless required for tree-sitter FFI

## Running Tests

```bash
cargo test --all               # All tests
cargo clippy -- -D warnings    # Lint
cargo fmt --check              # Format check
cargo tarpaulin                # Coverage (target: 89%)
```

## Module Structure

```
src/
├── lib.rs          # Library crate root
├── cli/            # CLI argument parsing (binary crate)
├── parser/         # Tree-sitter + language adapters (IR production)
├── analyzer/       # Diagnostics, flow tracing, boundary detection
├── graph/          # Persistent analysis graph, queries, cache
├── manifest/       # Output formatting (YAML, JSON, SARIF, summary)
└── config/         # Configuration loading and validation
```

## Architecture in Brief

Data-oriented, ECS-inspired (not ECS framework). Symbols are IDs in flat tables. Analyzers are functions that query the graph. Parser -> Graph -> Analyzer -> Manifest. The graph is the source of truth; manifests are exports.

## Issue Policy

Issues found during development are ALWAYS recorded as GitHub issues. Never ignored. Never deferred without tracking. Broken tests are fixed immediately or tracked with an issue.
