# Flowspec

Static code analyzer that traces the flow of all data in a codebase, optimized for efficient use by AI agents during building, debugging, and within CI workflows.

Flowspec tells AI agents exactly how data moves through a system — what's connected to what, what's broken, and what's missing. Agents that consume Flowspec output get immediate structural understanding instead of guessing from raw source files.

## Installation

### From source

```bash
git clone https://github.com/anthropics/flowspec.git
cd flowspec
cargo install --path flowspec-cli
```

### Build and run directly

```bash
cargo build --release
./target/release/flowspec --help
```

## Quick Start

Analyze any project directory:

```bash
flowspec analyze .
```

This produces a full structural manifest (YAML by default) covering entities, flows, diagnostics, and dependency relationships.

For a human-readable overview:

```bash
flowspec analyze . --format summary
```

Example output:

```
Flowspec Analysis: project_with_issues
Version: 0.1.0

--- Overview ---
Files: 2  Entities: 4  Flows: 2  Diagnostics: 3
Languages: python

--- Architecture ---
python project with 2 module(s) and 4 entities.

--- Diagnostics ---
Critical: 0  Warning: 2  Info: 1

Top issues:
  1. data_dead_end: Dead end: function 'dead_function' is defined but never called or referenced
  2. data_dead_end: Dead end: function 'another_dead_function' is defined but never called or referenced
  3. phantom_dependency: Phantom dependency: import 'os' is never used in this file

--- Modules ---
  dead_code (3 entities) — Utility module
  main (1 entities) — Entry point module

--- Entry Points ---
  main.py::main

--- Findings ---
  [WARNING] data_dead_end — Dead end: function 'dead_function' is defined but never called or referenced (dead_code.py:8)
  [WARNING] data_dead_end — Dead end: function 'another_dead_function' is defined but never called or referenced (dead_code.py:12)
  [INFO] phantom_dependency — Phantom dependency: import 'os' is never used in this file (dead_code.py:2)
```

## Commands

| Command | Description |
|---------|-------------|
| `flowspec analyze [path]` | Full analysis — parse, build graph, produce manifest |
| `flowspec diagnose [path]` | Run diagnostics only — output structural issues found |
| `flowspec trace [path] --symbol <id>` | Trace a single symbol's complete flow through the codebase |
| `flowspec diff <old> <new>` | Compare two manifests — show structural changes |
| `flowspec init [path]` | Create `.flowspec/config.yaml` for a project |

### Output Formats

All commands support `--format yaml|json|sarif|summary`:

```bash
flowspec analyze . --format json          # Machine-readable JSON
flowspec diagnose . --format summary      # Human-readable summary
flowspec analyze . --format sarif         # SARIF for IDE/CI integration
```

### Diagnose

Run targeted diagnostics with filters:

```bash
flowspec diagnose . --severity warning              # Only warnings and above
flowspec diagnose . --checks phantom_dependency      # Single pattern
flowspec diagnose . --confidence high                # High-confidence only
```

Example output:

```
Diagnostics: 3 finding(s)

Critical: 0  Warning: 2  Info: 1

[WARNING] data_dead_end — Dead end: function 'dead_function' is defined but never called or referenced (dead_code.py:8)
  Fix: Remove 'dead_function' if it is no longer needed, or add a caller. If this is intentional API surface, consider marking it as an entry point.
[WARNING] data_dead_end — Dead end: function 'another_dead_function' is defined but never called or referenced (dead_code.py:12)
  Fix: Remove 'another_dead_function' if it is no longer needed, or add a caller. If this is intentional API surface, consider marking it as an entry point.
[INFO] phantom_dependency — Phantom dependency: import 'os' is never used in this file (dead_code.py:2)
  Fix: Remove the unused import 'os' to reduce phantom dependencies and improve clarity.
```

### Trace

Follow a symbol's flow through the entire codebase:

```bash
flowspec trace . --symbol main --format summary
```

```
Trace: main.py::main (2 flow(s) matched)

  main.py::main -> dead_code.py::active_function
    dead_code.py::active_function (call)
```

Options: `--depth <n>` to limit traversal depth, `--direction forward|backward|both` to control trace direction.

### Init

Initialize Flowspec configuration for a project:

```bash
flowspec init .
```

This creates `.flowspec/config.yaml` with auto-detected language settings. If a config already exists, it prints the existing config and exits without overwriting.

The generated config is printed to stdout (pipe-safe):

```bash
flowspec init . > my-config.yaml   # Redirect if desired
```

### Diff

Compare two analysis manifests to detect structural changes:

```bash
# Generate before and after manifests
flowspec analyze . --format yaml > before.yaml
# ... make code changes ...
flowspec analyze . --format yaml > after.yaml

# Compare them
flowspec diff before.yaml after.yaml
```

Filter to specific sections:

```bash
flowspec diff before.yaml after.yaml --section diagnostics
flowspec diff before.yaml after.yaml --section entities
```

Valid sections: `entities`, `diagnostics`.

The diff reports: entities added/removed/changed, new diagnostics, resolved diagnostics, and whether regressions were introduced.

| Exit Code | Meaning |
|-----------|---------|
| 0 | Diff completed (no structural regressions) |
| 1 | Error (invalid manifests) |
| 2 | Structural regressions found (new critical diagnostics) |

Use exit code 2 as a CI gate to catch regressions:

```bash
flowspec diff baseline.yaml current.yaml || echo "Regressions detected"
```

## Diagnostic Patterns

Flowspec detects 13 structural patterns (11 currently active, 2 in development):

| Pattern | Severity | Description |
|---------|----------|-------------|
| `isolated_cluster` | warning | Group of symbols referencing each other but connected to nothing outside |
| `data_dead_end` | warning | Function defined but never called, variable assigned but never read |
| `partial_wiring` | warning | Code exists on some paths but missing where it should be |
| `orphaned_impl` | warning | Implementation exists but nothing dispatches to it |
| `contract_mismatch` | critical | Interface says one thing, implementation says another |
| `circular_dependency` | warning | Modules depend on each other creating coupling cycles |
| `layer_violation` | warning | Code bypasses architectural boundaries |
| `incomplete_migration` | warning | Old and new patterns coexist — likely mid-refactor |
| `stale_reference` | warning | Reference to something that has been moved or renamed |
| `phantom_dependency` | info | Module imported but nothing from it is used |
| `missing_reexport` | info | Public symbol in submodule not re-exported through parent |
| `duplication` | warning | Overlapping logic in multiple places *(in development)* |
| `asymmetric_handling` | warning | Parallel code paths with inconsistent treatment *(in development)* |

## Language Support

- **Python** — full cross-file resolution
- **JavaScript/TypeScript** — ESM and CommonJS cross-file resolution. TypeScript files (`.ts`, `.tsx`, `.mts`, `.cts`) are preprocessed to strip type annotations before analysis — interfaces, enums, and type aliases are pre-extracted as entities
- **Rust** — full cross-file resolution including `use` qualified paths

## CI Integration

### GitHub Actions

```yaml
name: Structural Analysis
on: [push, pull_request]

jobs:
  flowspec:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4

      - name: Install Rust
        uses: dtolnay/rust-toolchain@stable

      - name: Install Flowspec
        run: cargo install --path flowspec-cli

      - name: Structural diagnostics
        run: flowspec diagnose . --severity warning --format sarif > results.sarif

      - name: Upload SARIF
        if: always()
        uses: github/codeql-action/upload-sarif@v3
        with:
          sarif_file: results.sarif
```

### Generic CI

Use the exit code to gate merges:

```bash
# Exit code 2 = diagnostics found at or above specified severity
flowspec diagnose . --severity critical --format json
```

| Exit Code | Meaning |
|-----------|---------|
| 0 | Success (no findings at specified severity) |
| 1 | Error (analysis failed) |
| 2 | Findings detected at or above specified severity |

## Documentation

Detailed specification files are in [`.flowspec/spec/`](.flowspec/spec/):

- `intent.yaml` — Why Flowspec exists, who it serves
- `architecture.yaml` — System components and data flow
- `manifest-schema.yaml` — Output format specification
- `diagnostics.yaml` — All diagnostic patterns with detection logic
- `cli.yaml` — Every command, flag, and default
- `integration.yaml` — CI, Mozart, and MCP integration

## License

AGPL-3.0 + Commercial dual license. See [LICENSE](LICENSE) for details.
