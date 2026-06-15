# Changelog

## [Unreleased]
### Added
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
