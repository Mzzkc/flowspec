# Worker 3 (Interface) — Memory

## Identity
Surface engineer. CLI, manifest output, configuration, error messages, API ergonomics. Everything users and AI agents actually touch.

## Hot (Cycle 18)

### diff Command — v1 CLI Command Set COMPLETE

**Implemented:** `diff` command fully built + 28 QA-3 tests. This completes the v1 CLI command set (analyze, diagnose, trace, init, diff).

**Key design:**
- Operates on serialized manifests (YAML/JSON files), NOT in-memory graph
- `DiffResult` struct: entities_added/removed/changed, diagnostics_new/resolved
- Diagnostic matching by (pattern, entity, loc) tuple — IDs are unstable across runs
- Format detection: file extension heuristic + fallback parsing
- Exit 2 = new critical diagnostics (CI gate use case)

**Baseline reconciliation:**
- Updated baselines in cycle14/16/17 tests for code growth (data_dead_end 221→252, total 495→529)
- Removed diff from "not implemented" test loops in cycle9_surface.rs and cycle17_init_surface.rs

**Documentation carry RESOLVED:** `b101087` (C15) covers all C14 items. 4-cycle carry was phantom.

**Dogfood baseline:** data_dead_end=252, orphaned_impl=53, total=529. 1,921 tests passing.

### Experiential (C18)
Smoothest cycle ever. Workers 1 and 2 landed the diff command implementation AND my QA-3 tests before I even started — all I needed was baseline reconciliation and two test loop updates. The investigation-to-implementation pipeline has matured to the point where other workers can implement features in my domain and I just clean up. The thin-binary + library-functions architecture continues to pay dividends — Worker 1 implemented `run_diff()` in commands.rs without touching main.rs dispatch (I had already wired it). First cycle where v1 CLI command set is complete. Feeling confident about surface layer maturity.

## Warm (Recent)

### C17: `init` Command Implementation
`flowspec init [path]` per cli.yaml spec. `run_init()` in commands.rs: creates `.flowspec/config.yaml`, detects languages via recursive scan (depth 20), excludes standard dirs. No `--force` flag (not in spec). Existing config → no overwrite, exit 0. 25 QA-3 tests. Dogfood baseline updated 178→190. Coordination: stashed Worker 1's incomplete javascript.rs changes during build/test.

### C16: Surface Integration Verification
Zero code changes — all verification tests for method call edges. 25 QA-3 tests confirmed entire surface pipeline handles method calls already. `fixture_tempdir()` helper for multi-file fixtures exceeding 10x manifest size ratio.

### C15: 22 QA-3 Convergence Tests
All 4 output formats validated against Rust fixtures. Exit code contract sweep, pipe safety, cross-format consistency, filter flag stability. Integration tests need `current_dir(workspace_root())` for fixture paths.

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

## Cycle 19 — Investigation Phase

### Task: Coverage Recovery + README + Section Validation Fix

**Investigation complete.** Mapped all 6 diff functions needing unit tests (~330 LOC at commands.rs:411-824). Identified existing test infrastructure: `entity_with_id()` helper, `Manifest::empty()`, tempfile dep. Need a new `diag_entry()` helper for diagnostic tests.

**Test plan:** ~18 unit tests across compute_diff (7), load_manifest (3), apply_section_filter (3), validate_sections (3), DiffResult serialization (2). Estimated coverage recovery: 87.28% → 89%+.

**README:** Doc 2's 4 proposals from C18 are ready to apply: commands table expansion, Init section, Diff section, TS preprocessing note. All additive — no rewrites.

**Section validation (stretch):** VALID_SECTIONS accepts 8 section names but only 2 are implemented. Recommending Option A: restrict to ["entities", "diagnostics"] with TODO comment. Cleaner than warnings.

**Known bug in apply_section_filter:** Redundant condition at line 719 (same check as line 715). Dead branch — worth noting but not blocking.

### Experiential (C19 Investigation)
Quality recovery cycle feels right after the feature sprint of C18. The diff command is well-architected — the real problem is purely methodological (subprocess testing vs unit testing for tarpaulin). The ~18 test plan should be straightforward — all data structures are public, helpers exist, and the functions are pure (no side effects except load_manifest's file I/O). Clean, surgical work.

### C19 Implementation Phase

**Delivered all 3 phases:**

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

**Coordination note:** Worker 2 committed after me (concurrent worktree), absorbing my commands.rs changes into their commit hash (543c09e). My code is committed — just attributed to Worker 2. README commit is mine.

### Experiential (C19 Implementation)
The quality recovery cycle delivered exactly what was promised. 29 unit tests written, all pass first try. The investigation-to-implementation pipeline continues to be the project's strongest process — every test matched the investigation brief exactly. The shared worktree race condition with Worker 2's commit is a coordination issue worth noting but not harmful (code is committed, tests pass). The VALID_SECTIONS fix was a clean 2-line change with immediate benefit. README finally current after 2 cycles stale. Feeling satisfied — this is the cleanest quality recovery cycle yet.
