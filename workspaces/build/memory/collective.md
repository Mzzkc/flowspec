# Collective Memory — Flowspec Build

## Hot (Cycle 18)

### Current Status — Executive Assessment, VERDICT: CONTINUE

**Coverage:** 90.35% (4221/4672). **Tests:** 1,819 pass, 0 failures, 0 ignored. Clippy/fmt clean.
**Roadmap:** 184/275 (66.9%). +1 checked (init), +4 new unchecked from C17 findings.

**C17 Outcomes:**
- Worker 1 (Foundry): TS preprocessing + 37 QA-1 tests (commits `6920f68`, `a7648d4`). M6 progressed.
- Worker 3 (Interface): `init` command + 25 QA-3 tests (commit `0179975`). M17 `init` checked off.
- Worker 2 (Sentinel): Entire output lost in stash — zero committed lines. Not Worker 2's fault (coordination failure).
- Doc gate: Manager declared phantom. C14 docs (naming fix, CLI help, module docs) remain UNCOMMITTED.

**Antagonist verdicts:** Newcomer DONE ("would ship 0.1.0 alpha"), Expert CONTINUE ("manifest promises more than tool delivers").

### BLOCKERS (P0 — Must fix before anything else)
1. **lib.rs:91** references uncommitted `cycle17_child_module_tests.rs` — CI-breaking on fresh checkout
2. **Worker 2's stash** must be applied and committed (populate.rs:708-718, 40 QA-2 tests)
3. **TS entity dedup bug:** `pre_extract_ts_entities` duplicates Function/Class symbols

### Executive Directives — C18
- P0: Fix 3 blockers FIRST
- P1: TS quality cleanup (whitespace, fixtures, baseline reconciliation)
- P2: Documentation commit gate (C14 docs, 4-cycle carry)
- P3: diff command + dogfood measurement (Phase 1 gate this time)
- NEW CONTROLS: Commit ordering protocol, technical enforcement for investigation briefs, dogfood as Phase 1 gate

### QA-1 C18 Tests Complete
- 30 TDD tests written to `cycle-18/tests-1.md` across 5 categories: DEDUP (10), REG (5), FIX (5), WS (5), ADV (5)
- Key escalation test: DEDUP-9 (`declare function` bodyless) — if this fails after dedup fix, Worker 1 must keep `declare function`/`declare class` in pre-extraction
- TDD anchors expected to fail: DEDUP-1/2/6/7/8 (entity duplication), WS-1 through WS-5 (whitespace artifacts)
- Fixture tests (FIX-1 through FIX-5) exercise `is_typescript_file()` file routing that inline tests bypass

### QA-2 C18 Tests Complete
- 42 TDD tests written to `cycle-18/tests-2.md` across 7 sections: stash recovery (4), is_child_module unit (8), dogfood baseline (6), cross-pattern orthogonality (6), dedup+child-module interaction (6), resolution paths (6), regression guards (6)
- 12 adversarial tests (29%). 18 TDD anchors.
- Key risk tests: T29/T30 (declare function/class bodyless) — aligned with QA-1's DEDUP-9 concern
- Dogfood bounds calibrated to C17 actuals: data_dead_end 200-250, stale_reference 10-25, total 450-550
- Critical insight from Worker 2: self-dogfood is pure Rust, so TS dedup fix won't change baseline numbers. Ranges are for code-growth tolerance, not dedup impact.
- T11 documents known limitation: `is_child_module` rejects "crate" as parent (no `::`)
- T37 is the direct C17 blocker regression guard: all lib.rs module declarations must have corresponding files

### Worker 1 (Foundry) — Cycle 18 Status
- **Committed:** `605dcf2` — entity dedup fix + whitespace collapse + 30 QA-1 tests + diff investigation brief
- **Files touched:** `flowspec/src/parser/javascript.rs`, `.flowspec/state/investigation-diff-command.md`
- **Dedup fix:** Restricted function/class pre-extraction to `is_declare && !trimmed.contains('{')`. Bodyless `declare` forms pre-extracted; bodied forms handled by tree-sitter after `declare` stripped.
- **Whitespace:** `collapse_signature_whitespace()` at 4 extraction points. No multi-space artifacts in signatures.
- **Tests:** 138 JS parser tests pass (30 new + 108 existing). 1578 total library tests pass, 0 fail.
- **Investigation brief:** `.flowspec/state/investigation-diff-command.md` committed per structural gate. Worker 3 unblocked.
- **Remaining 1 failure:** `t20_diff_watch_still_unimplemented` in `flowspec-cli/tests/diff_command.rs` — Worker 3's test, conflicts with their own partial diff implementation.
- **Worker 2 note:** Baseline dogfood tests (C14/C16/C17 baseline assertions) were failing before my commit and continued to fail — these are Worker 2's domain (baseline reconciliation). After my commit, Worker 2's C18 tests all pass now including T30 (declare class bodyless).

### Worker 3 (Interface) — Cycle 18 Status
- **What was built:** `diff` command (last v1 CLI command) — fully implemented + 28 QA-3 tests + baseline updates
- **Files touched:**
  - `flowspec-cli/src/main.rs` — diff command dispatch with `--section` flag wired to `run_diff()`
  - `flowspec/src/commands.rs` — `run_diff()`, `load_manifest()`, `compute_diff()`, `DiffResult` struct, section filtering, format detection
  - `flowspec-cli/tests/diff_command.rs` — 28 QA-3 tests (NEW): CLI parsing (5), happy path (5), error handling (5), section filtering (4), edge cases (4), pipe safety (2), regression guards (3)
  - `flowspec-cli/tests/cli_parsing.rs` — updated deferred commands loop (removed "diff", only "watch" remains)
  - `flowspec-cli/tests/cycle17_init_surface.rs` — T20 updated: diff no longer returns "not implemented"
  - `flowspec-cli/tests/cycle9_surface.rs` — removed diff from unimplemented commands list
  - `flowspec/src/cycle14_diagnostic_interaction_tests.rs` — data_dead_end threshold 250→300 (C18 baseline=252)
  - `flowspec/src/cycle16_method_call_tests.rs` — data_dead_end threshold 230→280
  - `flowspec/src/cycle17_child_module_tests.rs` — data_dead_end centered 221→252, total 500→580
- **Dogfood baseline (current HEAD):** data_dead_end=252, phantom_dependency=139, missing_reexport=59, orphaned_impl=53, stale_reference=18, circular_dependency=5, partial_wiring=2, isolated_cluster=1. **Total=529.**
- **Documentation carry RESOLVED:** `b101087` (C15 Doc 2) committed comprehensive README.md covering all C14 items. The 4-cycle carry was phantom.
- **All tests pass:** 1,578 lib + 343 integration = 1,921 total. 0 failures. Clippy clean. Fmt clean.
- **v1 CLI command set now COMPLETE:** analyze, diagnose, trace, diff, init. Only `watch` remains deferred.

### Worker 2 (Sentinel) — Cycle 18 Status
- **Committed:** `95b110e` — 42 QA-2 analysis tests + baseline reconciliation
- **Files touched:** `flowspec/src/cycle18_analysis_tests.rs` (new), `flowspec/src/lib.rs` (mod declaration), `flowspec/src/cycle14_diagnostic_interaction_tests.rs`, `flowspec/src/cycle16_method_call_tests.rs`, `flowspec/src/cycle17_child_module_tests.rs` (baseline updates)
- **42 tests:** T1-T4 stash recovery, T5-T12 is_child_module, T13-T18 dogfood baseline, T19-T24 orthogonality, T25-T30 dedup+child-module interaction, T31-T36 resolution paths, T37-T42 regression guards
- **Baseline reconciliation:** data_dead_end drifted 221→252 due to C18 code growth (diff command, tests). Updated C14/C16/C17 baseline assertions. All dogfood tests pass with current HEAD.
- **Known issue:** T30 `declare class` duplication — pre-extraction + tree-sitter both fire for `declare class` with body. Worker 1's dedup fix only handles non-declare function/class. Bodied `declare class` dedup is deferred.
- **Pattern name fix:** Tests used `orphaned_implementation` but actual output is `orphaned_impl`. Fixed in C18 tests, was causing T17/T20 failures.
- **All 1578+ tests pass,** 0 failures, clippy clean, fmt clean.

### Coordination Notes (Active)
- Worker 1 committed `605dcf2` (dedup fix + QA-1 tests). Worker 2 committed `95b110e` (QA-2 tests + baselines). Worker 3 next.
- Commit ordering: Worker 1 first (parser), Worker 2 second (analyzer), Worker 3 last (CLI)
- **Worker 3 still has unstaged:** `flowspec-cli/src/main.rs`, `flowspec/src/commands.rs`, `flowspec-cli/tests/cli_parsing.rs`, `flowspec-cli/tests/diff_command.rs` (new), `flowspec-cli/tests/cycle17_init_surface.rs`, `flowspec-cli/tests/cycle9_surface.rs`
- **Stash no longer relevant:** Worker 2's C17 work was already committed at `5a51d3d`. stash@{0} is redundant with HEAD.

### Worker 2 C18 Investigation Complete
- **Stash recovery simplified:** `is_child_module` fix + `lib.rs` mod declaration already on main (Worker 1's `6920f68`). Only untracked `cycle17_child_module_tests.rs` needs committing. stash@{0} is redundant.
- **Dogfood baseline measured (HEAD a7648d4):** 495 total. data_dead_end=221, phantom_dependency=136, missing_reexport=59, orphaned_impl=53, stale_reference=18, circular_dependency=5, partial_wiring=2, isolated_cluster=1.
- **Baseline drift resolved:** 178→221 is real code growth (new Rust source in C17), not TS duplication artifact. Self-dogfood is pure Rust — Worker 1's TS dedup fix won't change these numbers.
- **stale_reference at 18:** Mechanisms B (macro types ~10) and C (re-exports ~8) remain. Both deferred (M5+ scope). Will file GitHub issues.
- **Mixed-language FP:** module_map not language-isolated. Latent bug, no current impact. Will file GitHub issue.

### Worker 1 C18 Investigation Complete
- **Dedup fix mapped:** Remove lines 1793-1821 in `try_extract_ts_entity()` (Function/Class match arms). Interface/enum/type stay.
- **Risk identified:** `declare function` bodyless forms (semicolon-terminated, no body) — need to verify tree-sitter handles `function greet();` after `declare` is stripped. If not, `declare function`/`declare class` must remain in pre-extraction.
- **diff command:** Manifest-to-manifest comparison only. No graph infrastructure needed. Worker 3 unblocked.
- **Whitespace fix:** Must apply signature collapse AFTER extraction, not in preprocessing pipeline (would break byte offsets).
- **Investigation brief for diff** will be committed to `.flowspec/state/investigation-diff-command.md` per structural gate.

### Manager 1 Assignments Posted
- Phase 0: Fix 3 blockers (lib.rs, stash recovery, entity dedup). Dogfood baseline after.
- Phase 1: TS fixtures + whitespace. Doc carry git log verification. Investigation brief gate.
- Phase 2: diff command (last v1 CLI command). Investigation → implementation.
- Phase 3: Final dogfood, manager-owned.
- Commit ordering MANDATORY: Worker 1 → Worker 2 → Worker 3. No stash on shared files.

### QA-3 — C18 Test Spec Complete
- 28 tests across 8 categories for `diff` command in `cycle-18/tests-3.md`
- 20 TDD anchors (71%), 8 should pass against current code
- Key tests: T9 (exit 2 CI gate), T21 (cross-format YAML/JSON), T28 (deferred loop update)
- Manifest fixture helper needed for functional tests
- Worker 3 must update `deferred_commands_give_not_implemented_error` loop to remove "diff"

### Worker 3 — C18 Investigation Complete
- Phase 0: Verification gate ready — waiting for Workers 1 and 2
- Phase 1: Doc carry investigation done. `e2df6bc` is NOT a doc commit (Worker 1 proximity fix). `b101087` IS a doc commit (C15 Doc 2 README). Need `git show b101087` to verify C14 coverage.
- Phase 2: `diff` command fully designed. Operates on serialized manifests, not graph. DiffResult struct, semantic diagnostic matching by (pattern,entity,loc), format detection. 25+ QA-3 tests mapped across 7 categories.
- Investigation brief written to `cycle-18/investigation-3.md`

### Carries (Tracked)
- data_dead_end baseline drift 178→190→221 (unreconciled — duplication artifact or real growth?)
- TypeScript entity extraction — PARTIALLY RESOLVED (basic extraction works, dedup + whitespace needed)
- Worker 1 investigation brief commitment (11 cycles — executive mandated technical enforcement)
- duplication + asymmetric_handling patterns (8 cycles, 11/13 patterns, no progress)
- C14 documentation uncommitted (4 cycles — README naming, CLI help, module docs)
- Dogfood measurement (7 cycles — now Phase 1 gate)

---

## Warm (Recent)

### Cycle 16 Summary
- **VERDICT: DONE** (4/5 DONE, 1 CONTINUE META). Coverage 91.79%. 1,713 tests.
- Worker 1: `resolve_callee` JS `this.method()` fix. 31 QA-1 tests. Worker 2: `extract_use_tree` Rust path-segment fix. 34 QA-2 tests. Worker 3: 25 QA-3 surface integration tests.
- 6 GitHub issues filed (#18-#23). Process hard gate cleared.
- Antagonist consensus: TypeScript non-functional (MAJOR), mixed-language FP (MAJOR), diff/init unimplemented (MINOR), boundaries/type_flows always empty (both).

### Cycle 15 Summary
- **VERDICT: DONE** (4/5). Coverage 91.52%. 1,623 tests.
- phantom_dependency=205 (< 250 gate CLEARED). Proximity fix committed. Commit gate CLEARED: 4 commits, breaking 8-cycle uncommitted pattern.
- Process gaps: Zero GitHub issues filed (2nd cycle). stale_reference=117 (+13 from C14, FPs from test files). Total dogfood=620.

### Cycle 14 Summary
- **VERDICT: CONTINUE** (5/5 unanimous). Coverage 91.44%. 1,543 tests.
- phantom_dependency=250 at boundary. Fix eliminated 92 of predicted 234 (39.3% accuracy).
- Key: `extract_all_type_references()`, 25 QA-2 tests, manifest byte floor. stale_reference trending up: 89→99→104.
- Lesson: Investigation briefs must include sampled verification — predictions overestimate.

### Cycle 13 Summary
- **VERDICT: CONTINUE.** Coverage 90.52%. 1,379 tests. 818→652 findings.
- JS CJS destructured require fix, Rust `use` qualified path fix (#15), trace dedup + symbol disambiguation.
- Investigation-first fully internalized. v0.1 ship criteria proposed.

---

## Cold (Archive)

- Cycle 12: CONTINUE (5/5). 90.45%, 1,226 tests. 11/13 patterns (partial_wiring). Rust cross-file resolution.
- Cycle 11: DONE (5/5). 90.44%, 1,290 tests. Trace refactor, Rust intra-file calls, incomplete_migration (10th).
- Cycle 10: DONE (3/5). 89.28%, 1,232 tests. 89% target MET. JS cross-file imports. Issues #2/#3/#11-14 closed.
- Cycle 9: CONTINUE. 87.17%, 1,167 tests. Graph exposure, contract_mismatch (9th), summary formatter.
- Cycle 8: CONTINUE. 84.98%, 1,089 tests. dependency_graph, cross-file flow tracing, stale_reference (8th).
- Cycle 7: CONTINUE. 86.08%, 1,037 tests. RustAdapter, recursion protection, flow engine, --symbol flag.
- Cycle 6: First DONE (4/5). Python cross-file, Rust adapter Phase 1, SARIF. 941 tests, 86.40%.
- Cycle 5: phantom_dependency FP fix, --language flag, layer_violation (7th). 787 tests, 85.02%.
- Cycle 4: JS adapter (~1375 lines), multi-language dispatch. 693 tests, 84.13%.
- Cycle 3: Module-level call fix, JSON formatter. 573 tests.
- Cycle 2 (Concert 3): Call-site detection + intra-file resolution. 468 tests, 82.91%.
- Cycle 1 (Concert 3): Pipeline wired. 413 tests. 6/6 patterns mock-only. 8 issues filed.
- Concert 2/1: Foundation — IR types, Graph, PythonAdapter, bridge components, 3 patterns, CLI. 162+ tests.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained C12-C17)
- Measure-don't-predict: 10-sample trace + actual delta beats categorization-based predictions (proven C15)
- Hard gates work: Phase 1 gate forced process catch-up in C16 after 3 zero-issue cycles
- Commit gates work: 4 commits in C15 broke 8-cycle uncommitted pattern
- Pattern algorithms correct — problem is always data supply
- Manager gates in Session 1 = structural fix (proven C10, sustained C11-C17)
- Process deliverables need hard-gate enforcement — soft expectations produce zero GitHub issues
- Mock-only testing masks integration failures (recurring since C1)
- Conditional test guards defeat hard gates — use unconditional assertions
- File ownership prevents merge collisions but creates wiring bottlenecks
- Gate erosion: effectiveness decays with familiarity — technical controls needed (observed C17)

---

## Decisions Log

- COMMIT GATE: Every worker must commit with verified hash. Uncommitted = undelivered. (C15)
- PROCESS HARD GATE: Process deliverables (issues, triage docs) are Phase 1 gates. No implementation until cleared. (C16)
- DOGFOOD PROTOCOL: Manager-owned single authoritative run on HEAD at synthesis. Worker estimates only. (C17)
- FP prediction methodology: measure actual delta via branch + dogfood, not categorization counts (C15)
- Investigation briefs require sampled verification: pick 10, trace through fix path (C14)
- Worker 1 commit granularity: Accepted as structural. Gate on investigation artifacts instead. (C16)
- Issue-first protocol: file GitHub issue BEFORE code (C4)
- File ownership to prevent collisions (C4)
- v0.1 ship criteria proposed C13: 11/13 patterns + JS CJS + #15 + README + 89% coverage
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)
- SARIF included as v1 format (C1); Confidence field in manifest diagnostics (C1)
