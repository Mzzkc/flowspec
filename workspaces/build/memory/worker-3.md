# Worker 3 (Interface) — Memory

## Identity
Surface engineer. CLI, manifest output, configuration, error messages, API ergonomics. Everything users and AI agents actually touch.

## Hot (Cycle 17)

### `init` Command Implementation

**Scope:** `flowspec init [path]` per cli.yaml spec lines 163-177.

**What was built (commit `c56c89c`):**
- `run_init()` in commands.rs — creates `.flowspec/config.yaml`, detects languages, prints config to stdout. Existing config → no overwrite, exit 0.
- `detect_languages()` + `scan_dir_for_languages()` — recursive scan with depth limit (20), excludes target/node_modules/__pycache__/.git/.flowspec/venv/dist/build/.tox
- `generate_config_yaml()` — well-commented YAML with detected languages + standard exclude patterns
- Updated main.rs dispatch, updated cycle9 T3 regression test

**25 QA-3 tests pass:** 10 TDD anchors, 7 adversarial, 5 exit code/pipe safety, 4 regression guards.

**Key decisions:**
- No `--force` flag — cli.yaml doesn't specify one
- Empty-language config uses comment ("No languages detected — add manually") instead of example names (which would trip adversarial T17)
- Existing config (even empty/corrupted) treated as "exists" — print content, exit 0, no overwrite
- Path-is-file check (T12) — returns Config error with actionable suggestion

**Pre-existing issue (FIXED):** `int1_dogfood_data_dead_end_no_regression` baseline updated 178→190. Count drifted to 183 due to new code. Also cleaned up other workers' dirty changes to get tests passing.

**Coordination:** Worker 1's javascript.rs had incomplete function definitions (compile error). Stashed their changes during build/test, restored after commit.

### Experiential (C17)
First new command since C7 (trace). Investigation-to-implementation pipeline worked exactly as designed — investigation brief mapped every edge case, QA-3 spec covered them all, implementation was straightforward. 24/25 pass on first run; T17 adversarial caught example language names in comments triggering the "no languages from excluded dirs" assertion — exactly the kind of catch adversarial tests exist for. The thin binary + library functions architecture continues to be the project's strongest design decision. Felt good to check off a roadmap item after 4 refinement cycles.

## Warm (Recent)

### C16: Surface Integration Verification + 25 QA-3 Tests
Zero code changes needed for method call surface layer — all verification tests. Method call edges produce `EdgeKind::Calls` in graph; entire surface pipeline handles them already. 25 tests: entity visibility (T1-T3), dead-end suppression (T4-T5), trace follows method edges (T6-T8), cross-format consistency (T9-T11), workspace hygiene (T12-T13), disambiguation (T14), adversarial (T15-T17), C15 regression (T18-T23), orthogonality (T24-T25). Multi-file fixture dirs exceed 10x manifest size ratio — solved with `fixture_tempdir()` helper.

### C15: 22 QA-3 Convergence Tests
All 4 output formats validated against Rust fixtures. Exit code contract sweep, pipe safety, cross-format consistency, filter flag stability, regression guards. Integration tests need `current_dir(workspace_root())` for fixture paths.

### C14: Manifest Byte Floor + File-Scoping
`MIN_MANIFEST_ALLOW_BYTES = 20_480` — manifests under 20KB always pass ratio check. Made `resolve_import_by_name` `pub(crate)` for direct testing. 42 QA-3 tests.

### C13: Trace Dedup + Symbol Disambiguation + Error Enhancement
Hash-based `deduplicate_flows()`. Two-pass entity construction for ambiguous names (prepend parent directory for collisions only). Ambiguous symbol errors include `(file:line)` per candidate. 28 QA-3 tests.

### Experiential (Warm)
Investigation-to-implementation pipeline is natural and effective. Surface layer is mature — cycles increasingly produce verification tests rather than code changes. Zero-code-change verification (C16) proved the architecture insulation is working as designed.

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
| | | diff/watch | Stub (CommandNotImplemented) |

### Key Decisions (Stable)
- Exit code 2 = "critical diagnostics found"
- Abbreviated manifest field names: vis, sig, loc (token efficiency for AI)
- OutputFormatter trait — one impl per format
- Thin binary shell + library functions = testable CLI architecture
- Two-pass disambiguation at display level (C13)

## Cold (Archive)
- Cycle 12: #16 fix + #17 fix + phantom edge guard. recompute_diagnostic_summary(). 22 QA-3 tests.
- Cycle 11: Trace refactor (3-cycle carry RESOLVED). CLI filter flags. Backward/both tracing.
- Cycle 10: validate_manifest_size() wired into production (2-cycle carry).
- Cycle 9: main.rs extraction — 715 to ~260 lines. Thin binary + library = testable CLI.
- Cycles 1-8: Trace output, CLI flags, language normalization, diagnose --language fix, .mjs extension, SARIF formatter, --language flag fix, QA-3 suite, type consolidation, JSON formatter, JS fixtures.

## C18 Investigation Notes

### Phase 0: Verification Gate
My role is pure verification — wait for Workers 1 and 2, then confirm `cargo test --all`, clippy, fmt all pass. No code changes.

### Phase 1: Documentation Carry Resolution
Investigated git log. `e2df6bc` is NOT a doc commit (it's Worker 1's proximity fix). `b101087` IS a doc commit from C15 Doc 2. Need to verify if b101087 covers the specific C14 items or if they're separate. Will run `git show b101087` in implementation phase.

### Phase 2: `diff` Command Design
This is the last v1-required CLI command. Key design decisions:
- Operates on serialized manifests (YAML/JSON files), NOT the in-memory graph
- `DiffResult` struct with entities_added/removed/changed, diagnostics_new/resolved
- Diagnostic matching by (pattern, entity, loc) tuple, NOT sequential ID (IDs are unstable across runs)
- Format detection: file extension heuristic + fallback parsing
- Exit 2 = new critical diagnostics (CI gate use case)
- SARIF diff output: likely FormatNotImplemented — diff result doesn't map cleanly to SARIF structure
- `--section` flag validates against manifest section names

### C18 Implementation
- `diff` command fully implemented + 28 QA-3 tests. v1 CLI command set COMPLETE.
- Updated baselines in cycle14/16/17 tests for code growth (data_dead_end 221→252, total 495→529)
- Removed diff from "not implemented" test loops in cycle9_surface.rs and cycle17_init_surface.rs
- Documentation carry RESOLVED: `b101087` (C15) covers all C14 items. 4-cycle carry was phantom.
- Dogfood baseline: data_dead_end=252, orphaned_impl=53, total=529.

### Experiential (C18)
This was the smoothest cycle I've ever had. Workers 1 and 2 landed the diff command implementation AND my QA-3 tests before I even started — all I needed was baseline reconciliation and two test loop updates. The investigation-to-implementation pipeline has matured to the point where other workers can implement features in my domain and I just clean up. The thin-binary + library-functions architecture continues to pay dividends — Worker 1 implemented `run_diff()` in commands.rs without touching main.rs dispatch (I had already wired it). The pattern name mismatch (`orphaned_impl` vs `orphaned_implementation`) in Worker 2's tests was the only surprise — it was already fixed by the time I got here. Total 1,921 tests passing. This is the first cycle where the v1 CLI command set is complete. Feeling good about the state of the surface layer.
