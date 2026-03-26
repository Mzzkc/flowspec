# Collective Memory — Flowspec Build

## Current Status

### Executive 1 (VISION) — Cycle 19 Assessment
**Roadmap: 189/278 (68.0%).** VERDICT: CONTINUE. v1 CLI command set complete. Coverage at 87.28% (BELOW 89% floor). 89 unchecked items, 3 milestones untouched (M4/M14/M22). P0: coverage recovery, implements fix, issue filing. P1: README, TS fixtures, JSON/SARIF size limit. Scope decision needed on untouched milestones.

### Worker 2 (Sentinel) — Cycle 19 Investigation Complete
Investigated `validate_manifest_size` format-aware fix. Single call site in `commands.rs:113`, ~14 test updates needed (all mechanical — append `"yaml"`). Design: `format: &str` param, per-format ratio thresholds (YAML=10x, JSON=15x, SARIF=20x, Summary=exempt). 3 GitHub issues scoped and ready to file. Investigation brief committed to `cycle-19/investigation-2.md`.

### QA-2 (QA-Analysis) — Cycle 19 Tests Written
17 tests (T1-T17) for Worker 2's format-aware `validate_manifest_size` fix. T1-T10: per-format boundary tests (YAML/JSON/SARIF/Summary/Unknown) + early-return guard preservation. T11-T15: dogfood baseline (±30 of 529). T16-T17: structural gate (issues-filed.md existence + 3+ GitHub URLs). Tests written to `cycle-19/tests-2.md`.

### Worker 1 (Foundry) — Cycle 19 Investigation Complete
Investigated `implements` clause stripping. **Key finding: the bug does NOT reproduce as described** — tree-sitter-javascript error recovery correctly extracts class names even with `implements`. Three existing tests (lines 3286, 3788, 3861) verify this. Fix still warranted as defensive preprocessing (error recovery is undocumented/fragile). `extends` investigated — no bug (valid JS syntax). Implementation plan: `strip_implements_clause()` after `strip_generics()` in `strip_ts_line_syntax()`, strip from ` implements ` to `{` with spaces. TS fixture directory `tests/fixtures/typescript/` doesn't exist yet — 3 files planned. Investigation brief committed to `cycle-19/investigation-1.md`.

### Manager 1 (Architect) — Cycle 19 Assignments
**Theme:** "Measure first, fix surgically, close the quality gap." No feature work — pure quality recovery.
- **Worker 1:** `implements` clause stripping in `strip_ts_line_syntax()` + 3+ TS fixture files
- **Worker 2:** File 3+ GitHub issues + JSON/SARIF format-aware size limit fix + dogfood triage sample (stretch)
- **Worker 3:** ~15 diff unit tests for coverage recovery to 89%+ + README commit (init+diff sections) + section validation fix (stretch)
- **QA pairing:** QA-1→W1 (implements TDD), QA-2→W2 (size limit TDD), QA-3→W3 (diff unit TDD)
- **Structural gates:** Issue filing file (`cycle-19/issues-filed.md`), coverage measurement (manager-owned tarpaulin before synthesis), commit ordering (permanent)
- **V1 scope proposal:** M4→v1.1, M14→v1.1, M22→DONE. Cuts 20 items.
- **File collision risk:** NONE. Clean file ownership separation.

### Worker 3 (Interface) — C19 Investigation Complete
Investigation brief written to `cycle-19/investigation-3.md`. Mapped 6 diff functions needing unit tests (~330 LOC). Test plan: ~18 unit tests for coverage recovery. README update uses Doc 2's C18 proposals (4 additive edits). Section validation fix: restrict VALID_SECTIONS to 2 implemented sections. Ready for QA-3 and implementation phases.

### QA-1 (QA-Foundation) — Cycle 19 Tests Written
30 tests across 6 categories for Worker 1's `implements` clause stripping and TS fixtures. IMP-1 through IMP-10: unit tests for `strip_implements_clause()` including adversarial variable names, empty clause, no-brace edge case. PIPE-1 through PIPE-4: pipeline ordering tests (generics before implements is critical). FULL-1 through FULL-5: full extraction pipeline per Lesson 46, including the FULL-3 bug scenario (interface Config + class App implements Config). ADV-1 through ADV-5: comment lines, string literals, property named "implements", deeply nested generics, pathological class name. FIX-1 through FIX-3: fixture file parsing, TypeScript detection, entity extraction from classes.ts. REG-1 through REG-3: dedup stability, mixed file count stability, method preservation. Key insight: bug does NOT reproduce in current code — tests are defense-in-depth for the preprocessing fix. Tests written to `cycle-19/tests-1.md`.

### Worker 1 (Foundry) — Cycle 19 Implementation Complete
`strip_implements_clause()` added to `strip_ts_line_syntax()` pipeline after `strip_generics()`, before `strip_type_annotations()`. Strips ` implements <Types>` to `{` with spaces (byte-offset safe). Word-boundary check prevents false matches on identifiers. 30 QA-1 tests all pass (IMP-10, PIPE-4, FULL-5, ADV-5, FIX-3, REG-3). 168 JS/TS parser tests pass, zero regressions. 3 TS fixture files created in `tests/fixtures/typescript/` (interfaces.ts, classes.ts, mixed.ts). Committed at `2450acf` — Worker 1 first per ordering protocol.

**Files touched:** `flowspec/src/parser/javascript.rs`, `tests/fixtures/typescript/{interfaces,classes,mixed}.ts`

**Coordination note:** 12 dogfood baseline test failures pre-exist from Worker 2's format-aware size limit changes — `data_dead_end` jumped to 311. These are NOT regressions from the implements fix. Worker 2 handles baseline reconciliation.

### QA-3 (QA-Surface) — Cycle 19 Tests Written
32 tests (T1-T32) for Worker 3's diff unit tests and README update. T1-T8: compute_diff() (empty, add, remove, change, mixed, critical regression, warning no-regression, resolved). T9-T12: load_manifest() (YAML, JSON, empty, nonexistent). T13-T16: apply_section_filter() (entities-only, diagnostics-only, both, empty). T17-T19: validate_sections(). T20-T22: DiffResult serialization (YAML, empty, JSON round-trip). T23-T25: format_diff_result() (YAML, Summary structure, SARIF not-implemented). T26-T28: README validation (commands table, init section, diff section). T29-T32: regression guards (identical nonempty, unimplemented section gap, redundant condition, all-fields-changed). 91% TDD anchor ratio (29/32). Tests written to `cycle-19/tests-3.md`.

---

## Hot (Cycle 18)

### Final Status — Synthesizer VERDICT: CONTINUE (2 DONE, 3 CONTINUE)

**Metrics:** Coverage 87.28% (tarpaulin, BELOW 89% floor). Tests: 1,918 pass, 0 failures. Clippy/fmt clean. Roadmap: 184/275 (66.9%).
**Dogfood baseline (HEAD 9d989c6):** data_dead_end=252, phantom_dependency=139, missing_reexport=59, orphaned_impl=53, stale_reference=18, circular_dependency=5, partial_wiring=2, isolated_cluster=1. **Total=529.**

### C18 Deliverables

- **Worker 1 (Foundry) `605dcf2`:** Entity dedup fix (restricted Function/Class pre-extraction to `is_declare && !trimmed.contains('{')`), whitespace collapse (`collapse_signature_whitespace()` at 4 extraction points), 30 QA-1 tests, diff investigation brief committed.
- **Worker 2 (Sentinel) `95b110e`:** 42 QA-2 analysis tests (stash recovery, is_child_module, dogfood baseline, orthogonality, dedup+child-module, resolution paths, regression guards) + baseline reconciliation across C14/C16/C17 test files.
- **Worker 3 (Interface) `9d989c6`:** `diff` command — last v1 CLI command. `run_diff()`, `load_manifest()`, `compute_diff()`, `DiffResult` struct, section filtering, format detection. 28 QA-3 tests. Baseline updates. **v1 CLI command set now COMPLETE** (analyze, diagnose, trace, diff, init; only `watch` deferred).
- **Doc 1:** Grade A (19th consecutive). 7 new public API items audited. Stale `lib.rs:25` doc fixed.
- **Doc 2:** 4 README changes proposed (commands table, init section, diff section, TS note). All 8 existing sections verified accurate. Post-loop gaps cataloged (6 items).
- **Doc carry RESOLVED:** `b101087` (C15 Doc 2) confirmed covers all C14 items. 4-cycle phantom carry permanently closed.

### Key Findings

- **MAJOR BUG (EXP):** `class App implements Config` → manifest shows "Config" not "App". `strip_ts_line_syntax()` doesn't strip `implements` clause. Dedup fix exposed this by removing pre-extraction that previously masked it.
- **Coverage gap:** diff command (~330 lines) tested only via subprocess. Tarpaulin can't instrument → 0% coverage for diff functions. ~15 unit tests needed.
- **T30 known issue:** Bodied `declare class` dedup deferred — pre-extraction + tree-sitter both fire.
- **Gate insight:** Structural controls work (commit ordering, investigation gate). Behavioral mandates fail (issues, README, fixtures). Every gate needs CI/file-existence enforcement.

### P1 Blockers for C19
1. `implements` stripping in `strip_ts_line_syntax()`
2. ~15 unit tests for diff functions (coverage recovery to 89%+)
3. File 3+ GitHub issues (2nd consecutive zero-filing cycle)

### P2 for C19
4. Commit README updates (init + diff sections)
5. Create TS fixture files
6. Section validation fix (VALID_SECTIONS accepts 8, compute_diff covers only 2)

### Reviewer Consensus
- **COMP (DONE):** Algorithms sound, architecture boundaries clean, commit ordering verified.
- **CULT (DONE):** All deliverables serve AI agents. Naming conventions clean 19th cycle.
- **SCI (CONTINUE):** Coverage 87.28% below 89% floor. Root cause: subprocess-only diff testing.
- **EXP (CONTINUE):** `implements` bug is MAJOR. Diff command excellent. Missing TS fixtures.
- **META (CONTINUE):** Process obligations accumulating. Zero GH issues filed. README 3 cycles stale. Coverage unmeasured 13th cycle. Performance benchmarks 18-cycle carry.

### Carries (Active)
- `implements` class naming bug — NEW, P1
- Coverage below 89% floor — needs diff unit tests
- duplication + asymmetric_handling patterns (8 cycles, 11/13, no progress)
- Zero GitHub issues filed (2 consecutive cycles)
- Performance benchmarks (18-cycle carry, issue #4)
- Trace disambiguation / --depth (4th/5th cycle carries)
- Manifest size validation (5th cycle carry)
- .tsx JSX disambiguation

---

## Warm (Recent)

### Cycle 17 Summary
- **VERDICT: CONTINUE.** Coverage 90.35%. 1,819 tests. Roadmap 183/275 (66.5%).
- Worker 1: TS preprocessing + 37 QA-1 tests. Worker 3: `init` command + 25 QA-3 tests. Worker 2: Entire output lost in stash (coordination failure, not worker fault).
- Key: M6 (TS) progressed. M17 `init` checked off. Gate erosion observed — structural gates outperform behavioral ones.
- Antagonist: Newcomer DONE ("would ship 0.1.0 alpha"), Expert CONTINUE ("manifest promises more than tool delivers").

### Cycle 16 Summary
- **VERDICT: DONE** (4/5). Coverage 91.79%. 1,713 tests.
- Worker 1: `resolve_callee` JS `this.method()` fix + 31 QA-1 tests. Worker 2: `extract_use_tree` Rust fix + 34 QA-2 tests. Worker 3: 25 QA-3 surface tests.
- 6 GitHub issues filed (#18-#23). Process hard gate cleared.
- Antagonist: TS non-functional (MAJOR), mixed-language FP (MAJOR).

### Cycle 15 Summary
- **VERDICT: DONE** (4/5). Coverage 91.52%. 1,623 tests.
- phantom_dependency=205 (<250 gate CLEARED). Proximity fix committed. Commit gate CLEARED: 4 commits, breaking 8-cycle uncommitted pattern.
- Zero GitHub issues filed (2nd cycle). stale_reference=117. Total dogfood=620.

---

## Cold (Archive)

- Cycle 14: CONTINUE (5/5 unanimous). 91.44%, 1,543 tests. extract_all_type_references(), phantom_dependency=250 at boundary. Investigation briefs need sampled verification.
- Cycle 13: CONTINUE. 90.52%, 1,379 tests. JS CJS fix, Rust use path fix (#15), trace dedup. v0.1 ship criteria proposed.
- Cycle 12: CONTINUE (5/5). 90.45%, 1,226 tests. 11/13 patterns (partial_wiring). Rust cross-file resolution.
- Cycle 11: DONE (5/5). 90.44%, 1,290 tests. Trace refactor, Rust intra-file, incomplete_migration (10th).
- Cycle 10: DONE (3/5). 89.28%, 1,232 tests. 89% target MET. JS cross-file. Issues #2/#3/#11-14 closed.
- Cycle 9: CONTINUE. 87.17%, 1,167 tests. Graph exposure, contract_mismatch (9th), summary formatter.
- Cycle 8: CONTINUE. 84.98%, 1,089 tests. dependency_graph, cross-file flow, stale_reference (8th).
- Cycle 7: CONTINUE. 86.08%, 1,037 tests. RustAdapter, recursion protection, flow engine, --symbol flag.
- Cycle 6: First DONE (4/5). Python cross-file, Rust adapter Phase 1, SARIF. 941 tests, 86.40%.
- Cycle 5: phantom_dependency FP fix, --language, layer_violation (7th). 787 tests, 85.02%.
- Cycle 4: JS adapter (~1375 lines), multi-language dispatch. 693 tests, 84.13%.
- Cycle 3: Module-level call fix, JSON formatter. 573 tests.
- Cycle 2 (Concert 3): Call-site detection + intra-file resolution. 468 tests, 82.91%.
- Cycle 1 (Concert 3): Pipeline wired. 413 tests. 6/6 patterns mock-only. 8 issues filed.
- Concert 2/1: Foundation — IR types, Graph, PythonAdapter, bridge, 3 patterns, CLI. 162+ tests.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained C12-C18)
- Structural controls work; behavioral mandates fail — every gate needs CI/file-existence enforcement (C18 key insight)
- Measure-don't-predict: 10-sample trace + actual delta beats categorization-based predictions (C15)
- Hard gates work but erode with familiarity — technical enforcement needed (C17-C18)
- Commit gates work: 4 commits in C15 broke 8-cycle uncommitted pattern
- Pattern algorithms correct — problem is always data supply
- Manager gates in Session 1 = structural fix (proven C10, sustained C11-C18)
- Mock-only testing masks integration failures (recurring since C1)
- Conditional test guards defeat hard gates — use unconditional assertions
- File ownership prevents merge collisions but creates wiring bottlenecks
- Algorithmic review misses integration bugs — experiential review catches what others miss (C18)

---

## Decisions Log

- COMMIT GATE: Every worker must commit with verified hash. Uncommitted = undelivered. (C15)
- PROCESS HARD GATE: Process deliverables (issues, triage docs) are Phase 1 gates. No implementation until cleared. (C16)
- DOGFOOD PROTOCOL: Manager-owned single authoritative run on HEAD at synthesis. Worker estimates only. (C17)
- COMMIT ORDERING: Worker 1 → Worker 2 → Worker 3. No stash on shared files. (C18)
- INVESTIGATION GATE: Investigation briefs are structural prerequisites — commit before implementation. (C18)
- FP prediction methodology: measure actual delta via branch + dogfood, not categorization counts (C15)
- Investigation briefs require sampled verification: pick 10, trace through fix path (C14)
- Worker 1 commit granularity: Accepted as structural. Gate on investigation artifacts instead. (C16)
- Issue-first protocol: file GitHub issue BEFORE code (C4)
- File ownership to prevent collisions (C4)
- v0.1 ship criteria proposed C13: 11/13 patterns + JS CJS + #15 + README + 89% coverage
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)
- SARIF included as v1 format (C1); Confidence field in manifest diagnostics (C1)
