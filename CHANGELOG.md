# Changelog

## [Unreleased]
### Added
- **Memory budget** (`--memory-limit`, `FLOWSPEC_MEMORY_LIMIT`, default 2G): every allocation is counted and each analysis stage boundary stops the run with `memory budget exceeded after <stage>` and exit 1 once the budget is crossed. Added after an uncapped run exhausted a 15 GB host on 2026-11-10.
- **Streaming Python reference index** (`flowspec/src/pyindex/`, design in `docs/design/streaming-index.md`): each file is parsed once and reduced to a per-file fact set persisted under `.flowspec/index/`; only the export table stays resident; resolution runs per file against it and emits posting lists. For Python-only analyses `trace`, `diagnose` and `analyze` all read the index: all thirteen diagnostic patterns, entities, flows, module summaries, boundaries and the dependency graph are computed from posting lists. Re-runs re-parse only changed files and re-resolve only their import closure. Callers are reported as certain or possible with file:line and a reason; an empty answer says what was searched. Module-level variables are definitions with reference sites. Measured 2026-11-11 (Ronin, 2 GB cap): see `docs/design/streaming-index.md` and the oracle in `scripts/oracle/`.
- **CI accuracy gate**: `flowspec-cli/tests/oracle_gate.rs` scores the release binary with the oracle on the committed fixtures under `tests/oracle/` and fails on any missed or wrongly claimed caller; `cargo tarpaulin --fail-under 87` enforces the coverage floor.
- **Index findings match the graph producer's contract**: entity ids are `file.py::name` (directory-prefixed only on collision); public top-level functions are never `data_dead_end` findings; a definition decorated by a project decorator counts as consumed; lambda parameters shadow outer names; decorator contract violations name the bare method.
- **Index fact files are keyed by path and content**: two identical files in different directories no longer share one fact set, which on a warm rebuild gave the second file the first file's path and module (`trace` then reported the symbol missing). Index version bumped to 3; stale indexes rebuild.
- **Concurrent index builds are safe**: each atomic write uses a temporary file unique to the process and call, so parallel builds of one index (parallel tests) never rename each other's half-written file.
- **Oracle scores traces by full root-relative symbol path**, so same-named fixture files in different directories are distinct cases.
- **Script-style imports resolve**: an import that is not a package path is tried relative to each ancestor directory of the importing file (`from sibling import f`, `from repository.repo import x` beside `api/`), so circular and layer fixtures laid out as scripts resolve like packages.
- **Oracle harness** (`scripts/oracle/`): `ast`-derived ground truth for callers and an adjudicator for diagnose findings, the basis for measuring false negatives and false positives in CI.
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
