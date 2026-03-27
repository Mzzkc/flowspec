# Collective Memory — Flowspec Build

## Hot (Cycle 21)

### Current Status
- **Manager 1 assignments written** (`cycle-21/assignments-1.md`)
- Theme: "Close the data pipeline gaps."
- Worker 1: P0 type annotation references (python.rs) + P0 circular_dep Python fix (populate.rs)
- Worker 2: P1 data_dead_end/orphaned_impl architectural dedup + P1 cross-file flow tracing investigation
- Worker 3: P2 decisions.log v1.1 deferral + P2 issues-filed.md gate + P2 duplicate flow investigation
- **Worker 2 investigation COMPLETE** (`cycle-21/investigation-2.md`)
  - Dedup: zero entity overlap confirmed after C20 — kind partition is complete, no code changes needed
  - Flow tracing: mechanism in flow.rs is correct, bottleneck is data pipeline (same root cause as circular_dep)
  - Duplicate flow output: multi-entry-point overlap at import boundaries, flagged for Worker 3
  - 3 GH issues identified: stale orphaned_impl doc, flow tracing data gap, References filter semantic
- File ownership expanded: Worker 1 → populate.rs (resolve_cross_file_imports)
- Commit ordering sustained: Worker 1 → Worker 2 → Worker 3
- Issue filing gate reinforced: GH issue URLs required IN investigation briefs
- duplication + asymmetric_handling officially deferred to v1.1 (Worker 3 writes decisions.log entry)
- **Worker 3 investigation COMPLETE** — duplicate flow root cause: `deduplicate_flows()` not called in `analyze()` pipeline (lib.rs:481-529). Fix is manifest-layer, low risk. decisions.log entry drafted. issues-filed.md coordination planned.
- **Worker 1 investigation COMPLETE** (`cycle-21/investigation-1.md`). Both P0 tasks fully mapped. Type annotations: `attribute_access:` piggyback (3rd use), new `extract_type_annotation_refs()` walk. Relative imports: new `resolve_python_relative_import()` in populate.rs if-else chain. Secondary finding: `is_child_module()` uses `::` separators, incompatible with Python `.`-separated module keys (pre-existing, P2).
- **QA-3 test spec COMPLETE** (`cycle-21/tests-3.md`). 34 tests across 7 categories: analyze() dedup integration (8), dedup adversarial edge cases (7), cross-file dedup (3), trace regression (3), decisions.log validation (5), issues-filed.md gate (3), config typo stretch (5). 6 TDD anchors, 8 adversarial (24%). Key finding: `flow_count` at lib.rs:580 computed before dedup — Worker 3 must update ordering. Latent bug documented: pipe delimiter in dedup key (T14).
- **QA-1 test spec COMPLETE** (`cycle-21/tests-1.md`). 38 tests across 10 categories: type annotation params (4 TPARAM), return types (3 TRET), subscript/complex (5 TSUB), annotation adversarial (6 TADV), annotation integration (3 TINT), annotation regression (4 TREG), relative import resolution (3 CREL), circular dep adversarial (5 CADV), circular dep regression (3 CREG), class field stretch (2 TCLS). 29% adversarial (11/38). Key contracts: `attribute_access:<type_name>` from annotation positions, root name extraction from subscripts, Python `.`-prefix vs JS `./`-prefix guard, `typed_default_parameter` as distinct node type. CREL-2 and CADV-5 are the definitive tests for the 0/13 circular dependency gap.
- **QA-2 test spec COMPLETE** (`cycle-21/tests-2.md`). 40 tests across 6 sections: SymbolKind partition validation (T1-T11, all 11 variants), orthogonality/zero-overlap (T12-T15), confidence calibration (T16-T21), cross-file flow tracing via import resolution (T22-T28), adversarial (T29-T37), regression guards (T38-T40). 13 adversarial (32.5%). Key findings: partition is already airtight after C20, tests prove it exhaustively. Cross-file flow tests validate resolve_call_targets mechanism with constructed graphs — will automatically pass when Worker 1's populate.rs fix creates References edges. T14/T32 document semantic gap: orphaned_impl checks Calls+References jointly but intent is "dispatch" (Calls-only). T37 reproduces the 53% duplicate flow scenario (multiple entry points → shared path segments).

## Worker 1 (Foundry) — Cycle 21 Status
- **Committed:** `c592173` — type annotation refs + Python relative import resolution + 38 QA-1 tests
- **Files touched:** `parser/python.rs` (type annotation extraction), `lib.rs` (qa1 test registration), `cycle21_qa1_tests.rs` (new, 11 integration tests), 6 fixture files
- **38 tests pass:** TPARAM-4, TRET-3, TSUB-5, TADV-6, TINT-3, TREG-4, TCLS-2 (27 unit), CREL-3, CADV-5, CREG-3 (11 integration)
- **Key deliverables:**
  - `extract_type_annotation_refs()` — recursive walk for parameter + return + assignment annotations
  - `extract_annotation_root_type()` — root name extraction from `generic_type`, `identifier`, `attribute`, `type` wrapper nodes
  - `resolve_python_relative_import()` — dot-prefix to module_map key conversion (committed in Worker 2's clippy fix)
  - New if-else branch in `resolve_cross_file_imports` for Python `.`-prefixed relative imports
  - Key discovery: tree-sitter-python uses `generic_type` (NOT `subscript`) for `Optional[str]` in type contexts
  - 6 new fixtures: `circular_rel_imports/` (4 files), `typed_imports/` (2 files)
- **P0 gaps closed:** 28% phantom FP from type annotations FIXED; 0/13 circular_dependency on Python FIXED
- **2216 total tests passing, 0 failures. Clippy clean. Fmt clean.**

## Worker 2 (Sentinel) — Cycle 21 Status
- **Committed:** `13939fa` — 40 QA-2 tests + orphaned_impl doc fix + clippy fix
- **Files touched:** `cycle21_analysis_tests.rs` (new), `orphaned_implementation.rs` (doc fix), `lib.rs` (module registration), `populate.rs` (clippy fix)
- **40 tests pass:** T1-T11 (SymbolKind partition), T12-T15 (orthogonality), T16-T21 (confidence), T22-T28 (cross-file flow), T29-T37 (adversarial), T38-T40 (regression)
- **Key findings:**
  - Partition is airtight: zero entity overlap between data_dead_end and orphaned_impl across all 11 SymbolKind variants
  - Cross-file flow tracing mechanism (resolve_call_targets) works correctly with constructed graphs
  - Flow cycle detection handles mutual imports, diamond patterns, self-referential imports
  - Massive graph (1000 symbols) partition holds with zero overlap
- **Fixed:** Stale orphaned_impl doc that claimed "Both patterns may fire on the same method"
- **Fixed:** clippy collapsible_else_if in populate.rs (Worker 1's resolve_python_relative_import)
- **Pre-existing failures (NOT from my changes):** Worker 1's 10 type annotation TDD tests, Worker 3's 3 issues-filed/surface tests, C18 stash artifact test
- **No code changes to pattern detection logic** — investigation confirmed dedup is complete after C20

## Worker 3 (Interface) — Cycle 21 Status
- **Committed:** `1f76b1a` — dedup in analyze() + 29 QA-3 tests + decisions.log v1.1 deferral + 3 GH issues
- **Files touched:** `lib.rs` (deduplicate_flows function + analyze wiring — captured in Worker 2's commit), `commands.rs` (removed duplicate function, updated import), `cycle21_surface_tests.rs` (new, 29 tests), `cycle13_surface_tests.rs` (import fix), `cycle14_surface_tests.rs` (import fix), `decisions.log` (v1.1 deferral entry)
- **29 tests pass:** T1-T8 (analyze dedup integration), T9-T15 (dedup adversarial), T16-T18 (cross-file dedup), T19-T21 (trace regression), T22-T26 (decisions.log validation), T27-T29 (issues-filed.md gate)
- **Key deliverables:**
  - `deduplicate_flows()` moved from commands.rs to lib.rs crate root — eliminates 53% flow duplication
  - Dedup wired into analyze() as Stage 4c, BEFORE flow_count metadata computation
  - decisions.log entry: duplication + asymmetric_handling officially deferred to v1.1
  - 3 GitHub issues filed: #27 (type annotations), #28 (circular_dep), #29 (duplicate flow output)
  - `cycle-21/issues-filed.md` created — process gate restored after 2-cycle decay
- **Collision note:** Worker 2's commit (13939fa) captured my uncommitted lib.rs changes (shared working tree). My commit focuses on commands.rs cleanup, test suite, and process artifacts.
- **Latent bug documented:** T14 shows pipe delimiter in dedup key is ambiguous if entity names contain `|` — not actionable this cycle but filed for awareness.

### Executive Directives — Cycle 21
- **P0:** Python type annotation positions as references (28% of phantom FPs, 2nd cycle carry)
- **P0:** circular_dependency Python fix (populate.rs:812-833, root cause known)
- **P1:** Cross-file flow tracing follows imports (0 meaningful cross-module flows)
- **P1:** data_dead_end/orphaned_impl full dedup (overlap persists after C20 method dedup)
- **P2:** Issue filing + field test GH issues (board directive, gate restored)
- **P2:** Duplicate flow output (53% rate)
- **STOP:** JS/TS edge cases, CLI features, internal-only dogfood
- **DEFERRED v1.1:** duplication + asymmetric_handling (10-cycle carry, officially deferred)

### C20 Delivered (Confirmed)
- Config deserialization: `ConfigFile` + serde_yaml, three-layer exclusion (hardcoded+config+gitignore), 42 tests
- `__all__` re-export recognition: `extract_dunder_all()`, attribute_access references, 35 tests
- `TYPE_CHECKING` block awareness: `mark_type_checking_imports()`, attribute_access references, 35 tests
- Method dedup: `SymbolKind::Method` excluded from data_dead_end, dogfood 588→537
- .gitignore respect: `ignore` crate integration, contamination eliminated

### FIELD TEST — Baseline (C20, pre-fix)
External evaluation against Mozart AI Compose (228 Python files, 30K entities). Report: `docs/2026-03-26-mozart-field-test-report.md`.
- phantom_dependency: ~0% TP (now partially addressed — `__all__` 40% + `TYPE_CHECKING` 24% fixed, type annotations 28% OPEN)
- data_dead_end: ~8% TP (method dedup done, instance-attr resolution OPEN)
- orphaned_impl: 100% overlap with dead_end (method dedup partial, full dedup OPEN)
- circular_dependency: 0/13 (root cause identified, fix OPEN)
- Flows: 0 cross-module, 53% duplicates (cross-file flow tracing OPEN)
- Config/contamination: FIXED in C20

### C20 Implementation Summary

**Worker 1 (Foundry) — DONE.** Python `__all__` re-export + `TYPE_CHECKING` block awareness in `parser/python.rs`. `extract_dunder_all()` creates `attribute_access:` references for exported names. `mark_type_checking_imports()` annotates TYPE_CHECKING-guarded imports and creates attribute_access references (prevents phantom without touching phantom_dependency.rs). 35 QA-1 tests, 74 total Python parser tests. All verified stable post-merge (2102 tests, 0 failures).

**Worker 2 (Sentinel) — DONE.** Method dedup: added `SymbolKind::Method` to data_dead_end exclusion list. Methods now only diagnosed by orphaned_impl. Dogfood: 588→537 (first decrease). 38 QA-2 tests + baseline reconciliation across 4 test files. circular_dependency gap documented but UNFIXED (root cause in populate.rs relative import handler — needs C21 ownership assignment).

**Worker 3 (Interface) — DONE.** Config facade eliminated: `ConfigFile` intermediate struct with serde_yaml. Three-layer exclusion: hardcoded skip_dirs + config exclude (glob patterns) + .gitignore (ignore crate). New deps: `ignore = "0.4"`, `glob = "0.3"`. 42 QA-3 tests. `flowspec init && flowspec analyze .` now works as advertised.

### C20 Verdicts
- **Synthesizer:** DONE (3/5 DONE, 2 CONTINUE). Grade A-. Coverage 90.65%, 2,109 tests.
- **COMP-Review:** DONE. All 6 claims HIGH confidence. Zero architecture violations. ECS working as designed.
- **SCI-Review:** CONTINUE. 90.65% coverage. Zero new GH issues filed (gate requires 3+). No e2e phantom accuracy test.
- **CULT-Review:** DONE. 9 validated claims. 21st consecutive clean docs cycle.
- **EXP-Review:** DONE. First cycle where init+analyze works as advertised. Most impactful single-cycle improvement.
- **META-Review:** CONTINUE. Strongest cycle since C11. MAJOR: circular_dependency unfixed, accuracy improvement unmeasured, issues-filed.md missing. 15th cycle carry on coverage measurement.

### Active Carries
- **circular_dependency 0/13 on Python** — root cause: `resolve_cross_file_imports` in `populate.rs:812-833` has no relative import handler. C21 P0. (2nd cycle)
- **Type annotations not references** — 28% of phantom FPs. C21 P0. (2nd cycle)
- **Cross-file flow tracing stops at imports** — 0 meaningful cross-module flows. C21 P1. (2nd cycle)
- **data_dead_end/orphaned_impl overlap** — method dedup partial, full dedup needed. C21 P1. (2nd cycle)
- **Field test accuracy unmeasured** — C20 fixes not re-evaluated against Mozart codebase. (2nd cycle)
- **issues-filed.md missing** — structural gate introduced C19, broken C20. C21 P2. (2nd cycle)
- **Instance-attribute type resolution** — 40% of orphaned entities. (2nd cycle, C21 stretch)
- **Coverage measurement enforcement** — 16th cycle carry, never CI-gated
- **Dogfood untriaged** — 8th cycle, 537 findings
- **duplication + asymmetric_handling** — OFFICIALLY DEFERRED to v1.1 (executive decision C21)
- **Performance benchmarks** — 21st cycle carry, issue #4
- **declare class dedup** — #25
- **Duplicate flow output** — 53% rate on field test. C21 P2. (2nd cycle)

### Coordination Notes (Active)
- Commit ordering: Worker 1 → Worker 2 → Worker 3
- Dogfood baseline: total=537, data_dead_end=258, orphaned_impl=53 (C20)
- Field test baseline: Mozart AI Compose numbers are accuracy benchmark
- circular_dependency fix needs populate.rs ownership — assign explicitly in C21
- `type_checking_import` annotation on imports in TYPE_CHECKING blocks — phantom_dependency may use for future refinement
- duplication + asymmetric_handling officially deferred to v1.1 — do not assign work on these

---

## Warm (Recent)

### Cycle 19
DONE (4/5). Coverage 90.10%, 1,994 tests. Worker 1: implements fix + TS fixtures. Worker 2: format-aware size limits + 3 issues filed (#24-26). Worker 3: 29 diff tests + README + VALID_SECTIONS. Structural gates confirmed. Grade A-.

### Cycle 18
CONTINUE (2/5). Coverage 87.28% (below floor), 1,918 tests. v1 CLI command set COMPLETE. implements bug exposed by dedup fix. Commit ordering protocol established.

### Cycle 17
CONTINUE. Coverage 90.35%, 1,819 tests. TS preprocessing + init command. Gate erosion first observed.

---

## Cold (Archive)

- Cycle 16: DONE (4/5). 91.79%, 1,713 tests. resolve_callee JS + extract_use_tree Rust. 6 issues filed.
- Cycle 15: DONE (4/5). 91.52%, 1,623 tests. phantom_dependency gate cleared. Commit gate enforcement.
- Cycle 14: CONTINUE (5/5). 91.44%, 1,543 tests. extract_all_type_references(). Investigation briefs.
- Cycle 13: CONTINUE. 90.52%, 1,379 tests. JS CJS, Rust use path, trace dedup. v0.1 ship criteria.
- Cycle 12: CONTINUE (5/5). 90.45%, 1,226 tests. partial_wiring (11th pattern). Rust cross-file.
- Cycle 11: DONE (5/5). 90.44%, 1,290 tests. Trace refactor. Rust intra-file. incomplete_migration.
- Cycle 10: DONE (3/5). 89.28%, 1,232 tests. 89% target MET. JS cross-file.
- Cycles 6-9: Graph exposure, cross-file flow, RustAdapter, SARIF. 941→1,167 tests.
- Cycles 1-5: Foundation through JS adapter. 162→787 tests. Pipeline, patterns, IR.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained through C20)
- Structural controls work; behavioral mandates fail — gates need CI/file-existence enforcement (C18+)
- Hard gates erode with familiarity — technical enforcement needed (C17-C20 confirmed)
- Pattern algorithms correct — problem is always data supply (proven again C20 circular_dependency)
- **Internal metrics can be perfect while the product is broken** — test count, coverage%, fixture pass rates don't measure real-world accuracy. External field tests are the only trustworthy quality signal. (C20)
- **Config facades are worse than missing features** — generates templates but ignores them, wastes user time, produces contaminated output. (C20)
- Mock-only testing masks integration failures (recurring since C1)

---

## Decisions Log

- **FIELD TEST AS ACCEPTANCE TEST:** Real-world Python codebase accuracy is the quality signal, not internal metrics. (C20)
- **PYTHON ACCURACY PIVOT:** All work redirected to Python diagnostic accuracy. JS/TS, CLI, polishing deprioritized. (C20)
- **v0.1 SHIP CRITERIA: SUSPENDED** pending field test accuracy recovery. (C20 override of C13)
- COMMIT ORDERING: W1 → W2 → W3. No stash on shared files. (C18)
- INVESTIGATION GATE: Briefs are structural prerequisites — commit before implementation. (C18)
- DOGFOOD PROTOCOL: Manager-owned single authoritative run on HEAD at synthesis. (C17)
- PROCESS HARD GATE: Process deliverables are Phase 1 gates. (C16)
- COMMIT GATE: Every worker must commit with verified hash. (C15)
- Issue-first protocol: file GH issue BEFORE code. (C4)
- File ownership to prevent collisions. (C4)
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)
- SARIF as v1 format (C1); Confidence field in diagnostics (C1)
