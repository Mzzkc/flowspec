# Worker 3 (Interface) — Memory

## Identity
Surface engineer. CLI, manifest output, configuration, error messages, API ergonomics. Everything users and AI agents actually touch.

## Hot (Cycle 20)

### Investigation — Config System + .gitignore + File Exclusion

**Phase 0 investigation complete.** Brief at `cycle-20/investigation-3.md`.

**Key findings:**
- `Config::load()` at `config/mod.rs:25-58` is a facade — detects config file exists but never reads contents. Returns empty struct always.
- `Config` struct has 2 fields (`config_path`, `languages`). `exclude` field missing entirely. Template from `init` generates `exclude` section that nothing reads.
- `analyze()` at `lib.rs:221-223` takes `_config: &Config` (underscore = unused). `discover_source_files()` at `lib.rs:698` has no config parameter.
- Hardcoded `skip_dirs` at `lib.rs:703-713` is the only exclusion mechanism. No `.gitignore` support.
- `ignore` crate (v0.4.25): Unlicense/MIT, AGPL-compatible, by BurntSushi. Perfect fit. Replaces custom `walk_dir` entirely.
- `serde_yaml` already in Cargo.toml. Only new dep: `ignore = "0.4"`.

**Implementation plan:** 4 steps — (1) Add serde Deserialize to Config + `exclude` field, (2) Wire exclude to `discover_source_files`, (3) Replace `walk_dir` with `ignore` crate walker for .gitignore respect, (4) Wire config languages as fallback.

**Attack surface for QA-3:** 25+ test scenarios identified across config deserialization, file exclusion, gitignore integration, and end-to-end pipeline.

### Implementation — Config Deserialization + File Exclusion + .gitignore

**Phase 1 implementation complete.** All 4 steps from investigation plan delivered.

**What I built:**
1. Config deserialization — `ConfigFile` intermediate struct with serde, `read_config_file()` helper with graceful degradation
2. File exclusion wiring — `discover_source_files` takes `&[String]` exclude patterns, merges with hardcoded skip_dirs
3. `.gitignore` respect — replaced `walk_dir` with `ignore::WalkBuilder`, respects all gitignore sources
4. Config languages fallback — priority chain CLI > config > auto-detect, adapter filter now uses `active_languages`

**Files modified:** `config/mod.rs` (complete rewrite), `lib.rs` (discover_source_files + analyze wiring), `Cargo.toml` (+ignore, +glob)

**Tests written:** 42 QA-3 tests — 14 in config/mod.rs, 28 in cycle20_surface_tests.rs
- All 10 Category 1 (config deser) TDD anchors pass
- All 9 Category 2 (file exclusion) tests pass
- All 9 Category 3 (gitignore) tests pass — including negation, nested, combined
- All 5 Category 4 (language filtering) tests pass
- All 5 Category 5 (integration) tests pass
- All 4 Category 6 (edge cases) tests pass

**Baseline reconciliation:** Fixed cycle16 T14 circular_dependency baseline (5→6), updated cycle16_surface T5 for Worker 2's Method dedup.

**Collision handling:** Worker 1 had already reverted my discover_source_files signature change (they committed first). I re-applied my changes on top of Worker 1 + Worker 2's code. Clean merge.

### Experiential (C20 Implementation)
This was the most impactful implementation cycle of the entire project. The config facade is gone — `Config::load()` now actually reads YAML. The `ignore` crate replaced 30 lines of custom recursion with a battle-tested walker that handles .gitignore, nested ignores, negation patterns, and symlinks. The three-source exclusion model (hardcoded + config + gitignore) is clean and each source works independently.

The `active_languages` fix in the adapter filter was a bonus discovery — config languages were being read but never wired to the adapter selection. A 2-line change that makes `languages:` in config.yaml actually do something.

42 tests written and all pass on first full run (after fixing 2 issues: T24 gitignore negation semantics and T29 adapter filter wiring). The investigation-to-implementation pipeline continues to produce first-attempt or near-first-attempt implementations. Feeling deeply satisfied — this is the fix that makes Flowspec usable on real codebases.

### Experiential (C20 Phase 0)
The investigation exposed how deep the facade goes — it's not just that config isn't read, it's that the entire data path from YAML file to file discovery has zero wiring. The `_config` underscore prefix in `analyze()` is an honest admission that nobody implemented this. The field test was right: we built the template generator (`init`) but never connected it to anything. This is the most impactful single fix I'll ever make on this project — 59% contamination elimination. Feeling focused and motivated. The `ignore` crate is a gift — exactly the right abstraction at exactly the right level.

## Warm (Cycle 19)

### Coverage Recovery + README + VALID_SECTIONS Fix

**Phase 0 — 29 unit tests for diff functions (coverage recovery):**
- T1-T8: `compute_diff()` — identical empty, entity add/remove/change, mixed changes, critical regression, warning no-regression, resolved diagnostic
- T9-T12: `load_manifest()` — valid YAML/JSON, empty file error, nonexistent path TargetNotFound
- T13-T16: `apply_section_filter()` — entities-only, diagnostics-only, both, empty clears all
- T17-T19: `validate_sections()` — all valid accepted, unknown rejected, empty accepted
- T20-T22: DiffResult serialization — YAML, empty clean, JSON round-trip
- T23-T25: `format_diff_result()` — YAML, Summary structure, SARIF FormatNotImplemented
- T29-T32: Regression guards — identical nonempty, unimplemented section, redundant condition, all-4-fields-changed
- Helpers: `diag_entry()` + `manifest_with()` for test construction

**Phase 1 — README update:**
- Commands table: added `diff` and `init` rows (5 commands total)
- Init section: auto-detection, no-overwrite, stdout pipe-safe
- Diff section: manifest comparison, `--section` flag, exit codes 0/1/2, CI gating example
- TS preprocessing note in Language Support

**Phase 2 (stretch) — VALID_SECTIONS fix:**
- Restricted from 8 section names to 2 (`["entities", "diagnostics"]`)
- Added TODO comment listing sections to add when compute_diff() expands
- Prevents silent empty output on `--section flows`

**Baseline reconciliation:** data_dead_end 252→311, total 529→588 (Worker 1 TS fixtures + implements fix).

**Dogfood baseline:** data_dead_end=311, total=588. 1,654+ tests passing.

**Coordination note:** Worker 2 committed after me (concurrent worktree), absorbing my commands.rs changes into their commit hash (543c09e). Code is committed — just attributed to Worker 2. README commit is mine (`d0c41dd`).

### Experiential (C19)
The quality recovery cycle delivered exactly what was promised. 29 unit tests written, all pass first try. Investigation-to-implementation pipeline continues to be the project's strongest process — every test matched the investigation brief exactly. The shared worktree race condition with Worker 2's commit is a coordination issue worth noting but not harmful. The VALID_SECTIONS fix was a clean 2-line change with immediate benefit. README finally current after 2 cycles stale. Feeling satisfied — cleanest quality recovery cycle yet.

## Warm (Recent)

### C18: diff Command — v1 CLI Command Set COMPLETE
`diff` command fully built + 28 QA-3 tests. This completed the v1 CLI command set (analyze, diagnose, trace, init, diff). Operates on serialized manifests (YAML/JSON files), not in-memory graph. `DiffResult` struct with entities_added/removed/changed, diagnostics_new/resolved. Diagnostic matching by (pattern, entity, loc) tuple. Exit 2 = new critical diagnostics (CI gate). Smoothest cycle — other workers could implement features in my domain and I just cleaned up. Commit `9d989c6`.

### C17: `init` Command
`flowspec init [path]` per cli.yaml spec. Creates `.flowspec/config.yaml`, detects languages via recursive scan (depth 20), excludes standard dirs. No `--force` (not in spec). Existing config → no overwrite, exit 0. 25 QA-3 tests.

### C16: Surface verification — zero code changes, 25 QA-3 tests confirmed method call pipeline.
### C15: 22 QA-3 convergence tests — all 4 output formats, exit code contracts, pipe safety.

### Experiential (Warm)
Investigation-to-implementation pipeline is natural and effective. Surface layer increasingly produces verification tests rather than code changes — architecture insulation working as designed. The thin binary + library functions architecture is the project's strongest design decision.

## Key Reference

### Files I Own
- `flowspec-cli/src/main.rs` — CLI binary (thin shell)
- `flowspec-cli/tests/` — 20+ test files
- `flowspec/src/commands.rs` — Extracted CLI logic
- `flowspec/src/manifest/json.rs`, `sarif.rs`, `summary.rs` — Formatters

### API Contract (Must NOT Change)
- CLI flag names, subcommand names, exit code semantics (0/1/2)
- Manifest section ordering (metadata → summary → diagnostics → entities → ...)
- OutputFormatter trait signature, Error type variants
- All 8 manifest sections always present even when empty

### Output Format & CLI Status
| Format | Status | Command | Status |
|--------|--------|---------|--------|
| YAML | Implemented | analyze | Fully implemented (+filter flags C11) |
| JSON | Implemented | diagnose | Fully implemented |
| SARIF | Implemented | trace | Fully implemented (forward/backward/both C11, dedup C13) |
| Summary | Implemented | init | Implemented (C17) |
| | | diff | Implemented (C18) — v1 COMPLETE |
| | | watch | Stub (CommandNotImplemented) |

### Key Decisions (Stable)
- Exit code 2 = "critical diagnostics found" / "new critical in diff"
- Abbreviated manifest field names: vis, sig, loc (token efficiency for AI)
- OutputFormatter trait — one impl per format
- Thin binary shell + library functions = testable CLI architecture
- Two-pass disambiguation at display level (C13)
- Diff operates on serialized manifests, not graph

## Cold (Archive)
- Cycle 14: Manifest byte floor + file-scoping. `MIN_MANIFEST_ALLOW_BYTES = 20_480`. 42 QA-3 tests.
- Cycle 13: Trace dedup + symbol disambiguation + error enhancement. 28 QA-3 tests.
- Cycle 12: #16 fix + #17 fix + phantom edge guard. recompute_diagnostic_summary(). 22 QA-3 tests.
- Cycle 11: Trace refactor (3-cycle carry RESOLVED). CLI filter flags. Backward/both tracing.
- Cycle 10: validate_manifest_size() wired into production (2-cycle carry).
- Cycle 9: main.rs extraction — 715 to ~260 lines. Thin binary + library = testable CLI.
- Cycles 1-8: Trace output, CLI flags, language normalization, diagnose --language fix, .mjs extension, SARIF formatter, --language flag fix, QA-3 suite, type consolidation, JSON formatter, JS fixtures.
