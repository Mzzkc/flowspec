# Collective Memory — Flowspec Build

## Current Status

### Worker 1 (Foundry) — C15 Implementation Complete
- **Proximity fix implemented in `resolve_import_by_name`** (populate.rs:502-547). Signature now takes `ref_line: u32`. Returns nearest preceding import by line number, with fallback to any match.
- **24 QA-1 tests passing** (cycle15_proximity_tests.rs): R1-R4 regression, D1-D6 core fix, A1-A5 adversarial, E1-E4 edge cases, I1-I3 integration, REG1-REG2 previous cycle guards.
- **Fixed QA-3's cycle15_convergence.rs** — undeclared `fixture` variable replaced with `rust_fixture_path()`.
- **Fixed Reference struct `id` field** missing in C15 test initializers (ir.rs gained ReferenceId in C14).
- **1623 tests, 0 failures, clippy clean, fmt clean.**
- **Files touched:** `flowspec/src/graph/populate.rs`, `flowspec/src/graph/mod.rs`, `flowspec/src/cycle15_proximity_tests.rs`, `flowspec/src/cycle14_surface_tests.rs`, `flowspec/src/lib.rs`, `flowspec-cli/tests/cycle15_convergence.rs`.

### Worker 2 (Sentinel) — Cycle 15 Investigation Complete
- **Dogfood verified:** 652 findings, all 8 pattern counts match C14 exactly. No drift.
- **Full triage completed:** All 8 categories classified with sampled evidence. ~78% FP rate (508/652).
- **Top FP mechanisms:** (1) `#[test]` functions invisible to analysis (~108 across data_dead_end + orphaned_impl), (2) path-segment imports (104 stale_reference, all FP), (3) import-name mismatch in `attribute_access:` (~160 phantom_dependency)
- **TPs confirmed:** circular_dependency (5/5 TP), isolated_cluster (1/1 TP), ~35 data_dead_end TPs, ~25 phantom_dependency TPs
- **GitHub issue roadmap:** 10 issues identified for FP categories, ready to file in Phase 3
- **QA-2 brief delivered:** 5 priority reproduction tests + edge cases documented in investigation-2.md
- **No code changes — investigation and triage only per assignment**

### Worker 3 (Interface) — Cycle 15 Investigation Complete
- **Phase 1 verified:** All 3 uncommitted changesets safe to commit. No file overlaps. 1,543 tests pass.
- **Commit order confirmed safe:** Doc-1 → Doc-2 → Worker 1. No merge conflicts possible.
- **Phase 3 ready:** GitHub issue evidence gathered for resolve_import_by_name + attribute_access convention.
- **Stray file flagged:** `flowspec/src/cycle14_type_reference_tests.rs` untracked — Worker 1 should include or remove.
- **phantom_dependency count:** Will remain 250 after Doc commits (doc-only changes, no code).

### QA-1 (QA-Foundation) — C15 Test Spec Delivered
- **24 TDD tests** across 6 categories for Worker 1's proximity-based `resolve_import_by_name` fix.
- **Core tests (D1-D6):** Should FAIL against current code, PASS after fix. Cover 2-5 duplicate imports.
- **Integration tests (I1-I3):** Full parse → populate → phantom_dependency::detect pipeline.
- **Critical safety gate (I3):** Genuinely unused import must STILL be detected. No true positive suppression.
- **Adversarial (A1-A5):** Over-resolution guards, wrong-scope prevention, boundary conditions.
- **Signature-agnostic:** Tests validate outcomes, not implementation details of the proximity algorithm.
- **Written to:** `cycle-15/tests-1.md`

### QA-2 (QA-Analysis) — C15 Test Spec Delivered
- **34 tests across 9 sections** in `cycle-15/tests-2.md`
- **Every FP category from Worker 2's triage covered:** 10 FP reproduction tests, 7 TP controls, 6 adversarial/edge cases, 3 dogfood regression guards, 3 cross-pattern interaction guards, 5 true negative/exclusion guards.
- **Key tests:** T4 (phantom import-name mismatch — dominant FP, ~160 findings), T7 (stale_reference path-segment — 100% FP), T11 (#[test] function dead end — ~88 FPs), T16 (glob re-export missing_reexport — ~20 FPs), T20 (method dispatch orphaned_impl — ~24 FPs).
- **Dogfood baseline locked:** T29-T31 assert exact C14 counts (652 total, per-pattern, confidence distribution).
- **Cross-pattern guards:** T32 (phantom/stale orthogonality), T33 (dead_end/orphaned_impl domain boundary), T34 (fix meta-regression).

### QA-3 (QA-Surface) — C15 Test Spec Delivered
- **22 tests across 6 categories** in `cycle-15/tests-3.md`
- **Primary new coverage:** Rust fixture targeting for all 4 output formats (T1-T7). Existing CLI tests only use Python fixtures — Worker 1's changes are Rust-specific.
- **Regression guards:** Byte floor (C14), 8-section manifest (C3), no-unreachable (C6/C9), confidence field (C1), filter flags (C11).
- **All 22 expected to PASS** — convergence cycle, no TDD pre-fail anchors.
- **Post-Phase-2 rerun planned:** If Worker 1's phantom_dependency fix lands, T1-T7 will be re-run against Rust fixtures.

### Manager 1 (Architect) — Cycle 15 Assignments Written
- **Theme:** "Commit, close, ship."
- **Phase 1 (HARD GATE):** Commit all C14 outstanding work. Order: Doc-1 → Doc-2 → Worker 1. Three hashes required before Phase 2.
- **Phase 2 (HARD GATE):** Worker 1 traces 10 phantom_dependency FPs, implements fix, measures delta via dogfood. Worker 2 triages all 652 dogfood findings.
- **Phase 3:** GitHub issues for FP categories. resolve_import_by_name documentation issue.
- **Methodology change enforced:** No categorization-based predictions. Measure actual delta only.
- **Coordination:** Doc-1 → Doc-2 → Worker 1 commit ordering to avoid conflicts on shared files.

### Executive 1 (VISION) — Cycle 15 Assessment
- **Roadmap: 173/256 (67.6%).** +1 net from C14.
- **VERDICT: CONTINUE.** 1 v0.1 blocker remains: phantom_dependency < 250 (currently 250).
- **CRITICAL:** 3 C14 deliverables uncommitted. Board flagged. Commit gate mandated for all workers.
- **C15 P0:** (1) Commit all C14 outstanding work, (2) Get phantom_dependency below 250.
- **Board directive:** All deliverables committed to project repo. Workspace files are not deliverables.
- **Methodology change:** No more categorization-based FP predictions. Measure actual delta via branch + dogfood diff.

---

## Hot (Cycle 14)

### Synthesis (Manager 1) — Verdict: CONTINUE (5/5 unanimous)
- **Coverage end:** 91.44% (3769/4122). +0.53pp from start. 89% EXCEEDED. 8-cycle tarpaulin carry RESOLVED.
- **BLOCKER:** phantom_dependency=250, gate requires <250. Off by 1. Investigation predicted -234, actual -92 (2.5x overestimate).
- **CLEARED:** Manifest byte floor (committed), README fix, module docs, stale_reference root-caused, cli.yaml synced.
- **UNCOMMITTED:** Worker 1 P0 code, Doc-2 changes, Doc-1 changes.
- **Next cycle focus:** (1) Investigate resolution gap — why 142 predicted FPs survived, (2) Fix to get <=249, (3) Commit all code, (4) File GitHub issue for resolve_import_by_name.
- **Methodology change:** Investigation briefs must include sampled verification (pick 10, trace each through fix path).

### Reviewer Consensus — All 5 Reviewers: CONTINUE
- **1,543 tests** (working tree), 1,303 committed. 0 failed, clippy/fmt clean. +91 tests from C13.
- **Dogfood verified (all reviewers):** 652 total findings. phantom_dependency=250, stale_reference=104, data_dead_end=178, missing_reexport=59, orphaned_impl=53, isolated_cluster=1, circular_dependency=5, partial_wiring=2.
- **phantom_dependency at boundary:** Target "<250", actual 250. Fix eliminated 92 of predicted 234 (39.3% accuracy). Needs executive ruling or 1 more FP fix.
- **stale_reference trending up:** 89->99->104 across C12-C14. Root-caused (path-segment imports, all FPs from test files). Not a v0.1 blocker.
- **Worker 1 code UNCOMMITTED:** P0 deliverable (`extract_all_type_references` + 24 tests) in working tree only. 8th cycle of bundled commit pattern.
- **Coverage: 8th cycle unmeasured (no tarpaulin-end artifact). 4th cycle without tarpaulin measurement.**
- **No GitHub issue filed for resolve_import_by_name** per assignment requirement.

### Worker Deliverables — Cycle 14

**Worker 1 (Foundry):** Investigation + Phase 2 implementation. `extract_all_type_references()` in rust.rs. 342 phantom_dependency confirmed, 4 sub-patterns identified (type names 234, module path segments 68, re-exports 27, function/item names 40). Fix targeted sub-pattern 1 only, achieved 92 reduction (342->250). Phase 2 entirely in rust.rs, no populate.rs collision.

**Worker 2 (Sentinel):** 25 QA-2 diagnostic tests implemented and committed (`e4a37cf`). Covers stale_reference, phantom_dependency, data_dead_end, cross-pattern, and dogfood regression. Key invariant: phantom checks EDGES, stale checks RESOLUTION STATUS — orthogonal. stale_reference regression root-caused: all 10 new findings are FPs from C13 test files.

**Worker 3 (Interface):** Manifest byte floor (`MIN_MANIFEST_ALLOW_BYTES = 20_480`) committed (`817f5c0`). File-scoping verified already correct. 42 QA-3 tests. `resolve_import_by_name` made `pub(crate)` for testing. 1303 tests pass.

**Doc-1 (Doc-API):** 5 core module `//!` docs updated with pipeline context. 2 stale references fixed. M23 at ~88%. Remaining: architecture doc, algorithm writeups (post-loop).

**Doc-2 (Doc-Usage):** README `orphaned_impl` naming fixed. All 8 cli.yaml discrepancies from C12 audit resolved. README examples refreshed.

**QA-1:** 25 TDD tests for type-name sub-pattern. QA-2: 28 test spec for diagnostic interactions. QA-3: 42 tests for byte floor and file-scoping.

---

## Warm (Recent)

### Cycle 13 Summary
- **Verdict: CONTINUE.** Coverage 90.52%. 1,379 tests. Dogfood triage completed as manager gate (4-cycle carry resolved). 818->652 findings after fixes.
- **Key deliverables:** JS CJS destructured require fix (Worker 1), Rust `use` qualified path fix for Issue #15 (Worker 1), trace dedup + symbol disambiguation (Worker 3), #17 closed. M4/M14 investigation briefs delivered (Worker 2).
- **v0.1 ship criteria proposed:** 11/13 patterns + JS CJS + Issue #15 + README + 89% coverage. M4/M14/remaining patterns deferred to v0.2.
- **QA-1 critical finding:** Dotted callee name approach incompatible with `resolve_callee`. Worker 1 used `attribute_access:` path instead.
- **Process:** Investigation-first fully internalized. Bundled commit pattern continues (6+ cycles).

### Cycle 12 Summary
- **Verdict: CONTINUE (5/5).** Coverage 90.45%. 1,226 tests. 11/13 patterns (partial_wiring delivered).
- **Key deliverables:** Rust cross-file resolution (P0, 6-cycle gap closed), partial_wiring pattern (11th of 13), #16/#17 fixes, CLI --help audit (8 discrepancies), performance investigation brief.
- **818 findings untriaged (3-cycle carry, BLOCKER).** Escalated to manager gate.
- **Lesson:** Verification-last needs same enforcement as investigation-first. Enforcement hierarchy proven: manager gate > Phase 1 hard gate > Phase 2+ > worker assignment.

---

## Cold (Archive)

- Cycle 11: DONE (5/5). 90.44%, 1,290 tests. Trace refactor, Rust intra-file calls, incomplete_migration (10th). Investigation-first internalized.
- Cycle 10: DONE (3/5). 89.28%, 1,232 tests. 89% target MET. JS cross-file imports. 587 findings triaged. Issues #2/#3/#11-14 closed.
- Cycle 9: CONTINUE. 87.17%, 1,167 tests. Graph exposure, contract_mismatch (9th), summary formatter wired (9-cycle carry).
- Cycle 8: CONTINUE. 84.98%, 1,089 tests. dependency_graph, cross-file flow tracing, stale_reference (8th).
- Cycle 7: CONTINUE. 86.08%, 1,037 tests. RustAdapter, recursion protection, flow engine, dependency graph, --symbol flag.
- Cycle 6: First DONE (4/5). Python cross-file, Rust adapter Phase 1, SARIF. 941 tests, 86.40%.
- Cycle 5: phantom_dependency FP fix, --language flag, layer_violation (7th). 787 tests, 85.02%.
- Cycle 4: JS adapter (~1375 lines), multi-language dispatch. 693 tests, 84.13%.
- Cycle 3: Module-level call fix, JSON formatter. 573 tests.
- Cycle 2 (Concert 3): Call-site detection + intra-file resolution. 468 tests, 82.91%.
- Cycle 1 (Concert 3): Pipeline wired. 413 tests. 6/6 patterns mock-only. 8 issues filed.
- Cycle 2 (Concert 2): Bridge components delivered.
- Cycle 1 (Concert 1): Foundation — IR types, Graph, PythonAdapter, 3 patterns, CLI. 162+ tests.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained C12-C14)
- Verification-last fails — needs same enforcement as investigation-first (learned C12)
- Pattern algorithms correct — problem is always data supply
- Manager gates in Session 1 = structural fix (proven C10, sustained C11-C14)
- Hard gates work: Phase 1 gate forced trace carry resolution (C11)
- Investigation briefs must include sampled verification — predictions overestimate (learned C14, 2.5x overcount)
- Mock-only testing masks integration failures (recurring since C1)
- Conditional test guards defeat hard gates — use unconditional assertions
- Bundled commits are structural (8+ cycles) — needs workspace isolation
- File ownership prevents merge collisions but creates wiring bottlenecks

---

## Decisions Log

- COMMIT GATE: Every worker must commit with verified hash. Uncommitted = undelivered. Board directive (C15)
- FP prediction methodology: measure actual delta via branch + dogfood, not categorization counts (C15)
- Investigation briefs require sampled verification: pick 10, trace through fix path (C14)
- Verification-last needs Phase 2 hard gate enforcement (C12)
- Phase 1 hard gates force carry resolution (proven C11)
- Investigation-first mandate fully internalized (C11)
- Manager gates execute in separate first session (C8, proven C10-C14)
- Phase 3 hard gates need unconditional assertions (C7)
- Pipeline wiring = explicit deliverable (C7)
- Issue closure = manager hard gate with CI enforcement (C7)
- Issue-first protocol: file GitHub issue BEFORE code (C4)
- File ownership to prevent collisions (C4)
- Investigation-before-implementation mandate (C2)
- SARIF included as v1 format (C1)
- Confidence field in manifest diagnostics (C1)
- Thin slice strategy over breadth-first (C1)
- v0.1 ship criteria proposed C13: 11/13 patterns + JS CJS + #15 + README + 89% coverage
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)
