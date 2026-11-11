# Changelog

## [Unreleased]
### Added
- **Memory budget** (`--memory-limit`, `FLOWSPEC_MEMORY_LIMIT`, default 2G): every allocation is counted and each analysis stage boundary stops the run with `memory budget exceeded after <stage>` and exit 1 once the budget is crossed. Added after an uncapped run exhausted a 15 GB host on 2026-11-10.
- **Python reference index**: `trace` on a Python symbol is answered by a dedicated resolver (`flowspec/src/pyrefs.rs`) with scopes, imports, class hierarchy and receiver typing; callers are reported as certain or possible with file:line, and an empty answer says what was searched. 380/380 certain direct callers on the Marianne corpus (was 309/380).
- **Cross-file flow tracing** (CF): `import m; m.f()` now traverses module boundaries — flowspec is useful on real multi-file codebases.
- **Dotted-import call resolution** (A1): `module.func()` calls create cross-file call edges.
- **Config validation**: warns on unknown top-level config keys (e.g. `analysis.ignore` → suggests `exclude`).
### Fixed
- **Multi-segment directory excludes** now filter contents (`tests/fixtures/` etc. were silently ignored → dogfood 1367→597 findings, test-pollution eliminated).
- 5 pre-existing cycle19/21 `issues-filed.md` process-gate tests re-homed to `#[ignore]` (gitignored workspace artifacts).

## 0.1.0
- Static data-flow analyzer (Python, JS/TS, Rust); 13 diagnostic patterns.
- CLI: `analyze`, `diagnose`, `trace`, `diff`, `init`; formats: yaml/json/sarif/summary.
- Exit-code contract: 0 clean / 1 error / 2 findings (CI gate).
