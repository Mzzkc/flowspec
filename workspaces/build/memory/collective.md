# Collective Memory — Flowspec Build

## Current Status

### Executive 1 (VISION) — Cycle 13 Assessment
- **Roadmap ~167/250 (66.8%).** +2 completed (Rust cross-file, partial_wiring). +6 new items from C12 antagonist findings and board directives. 83 items remaining.
- **Coverage:** 90.45%. Target exceeded 4th consecutive cycle.
- **Tests:** 1,226 verified, 0 failures, clippy/fmt clean.
- **Patterns:** 11/13. 2 remaining: duplication, asymmetric_handling (Very Hard).
- **Dogfood triage: 4-CYCLE CARRY.** Escalated from worker deliverable to MANAGER GATE. No worker assignments until dogfood runs.
- **Board directive: README.md required.** No external documentation exists. P1 this cycle.
- **JS CJS cross-file resolution BROKEN** (antagonist-2 BLOCKER). Destructured `require()` produces zero cross-file edges.
- **Rust `use` path phantom dependency FP** (antagonist-2 MAJOR, Issue #15). Standard Rust idiom flagged incorrectly.
- **Open issues:** #1, #4, #5, #6, #15.
- **3 milestones at 0%:** M4, M14, M22. Investigation briefs mandated P3.
- **v0.1 ship criteria: 5th cycle undefined.** Decision required this cycle.

## Hot (Cycle 12)

### Synthesis — VERDICT: CONTINUE (5/5 unanimous)
- **Coverage:** 90.45% (3597/3977). UP +0.09pp from 90.36% start. 89% target exceeded.
- **Tests:** 1,226 verified (0 failures, clippy/fmt clean). Prior 1,379 count was stale from retry state.
- **Patterns:** 11/13 (84.6%). partial_wiring delivered. Remaining 2: duplication, asymmetric_handling (Very Hard).
- **Findings:** 818 (up from 707 C11). Untriaged. phantom_dependency 425, data_dead_end 178, stale_reference 89, missing_reexport 59, orphaned_impl 53, isolated_cluster 7, circular_dependency 5, partial_wiring 2.
- **BLOCKER:** Dogfood triage not completed — 3-cycle carry. Cycle hard gate not met.

### Key Deliverables
- **Rust cross-file resolution (P0):** mod tree path-based keys, from: annotations, Phase 5 transitive calls. 28 tests. 6-cycle gap with Python/JS closed.
- **partial_wiring pattern (P1):** 11th of 13. Import-Call Gap Analysis. 5-layer FP mitigation. 42 tests.
- **#16 fix (P2):** recompute_diagnostic_summary() after apply_diagnostic_filters(). Both primary and secondary staleness resolved.
- **#17 fix (P2):** Command-agnostic error listing all 13 valid patterns.
- **M23 docs ~75%:** Full code audit, public type docs, module-level docs. Remaining: per-variant enum docs, architecture doc, algorithm writeups.
- **CLI --help audit:** 8 discrepancies found (2 HIGH: trace --depth/--direction defaults). Artifact: cycle-12/cli-help-audit.md.
- **Performance investigation brief:** criterion recommended, 4 spec targets mapped, 7-step implementation plan. Addresses Issue #4 (12-cycle carry).

### Active Carries (into C13)
- **Dogfood triage (4-cycle carry, BLOCKER):** Escalated to manager gate. No worker assignments until completed.
- **Tarpaulin end measurement:** Manager runs it, no longer worker-assigned.
- **Rust fixture files (3-cycle carry):** QA-1 Phase 2 not created.
- **v0.1 ship criteria (5th cycle):** Decision required.
- **2 remaining patterns:** duplication, asymmetric_handling — may need IR extensions.
- **JS CJS cross-file broken (NEW):** Antagonist-2 BLOCKER.
- **Rust use-path phantom FP (NEW escalation):** Antagonist-2 MAJOR, Issue #15.
- **README.md (NEW):** Board directive + antagonist-1 BLOCKER.

### Process & Experiential
- Investigation-first fully internalized (5th cycle).
- Verification-last converted from worker assignment to manager gate (lesson from 4-cycle failure).
- Enforcement hierarchy proven: manager gate (100%) > Phase 1 hard gate (~95%) > Phase 2+ (~60%) > worker assignment (~70%).
- Board flagged documentation gap — correct assessment. No external-facing docs exist.

---

## Warm (Recent)

### Cycle 11 Summary
- **Verdict: DONE (5/5 unanimous).** Coverage 90.44% (3385/3743). 1,290 tests. +58 from C10.
- **Key deliverables:** Trace refactor (FROM semantics, 3-cycle carry resolved), Rust intra-file call resolution (5-cycle gap resolved), incomplete_migration pattern (10th of 13), CLI filter flags, backward/both tracing, .cjs extension fix.
- **Process:** Investigation-first mandate fully internalized. Phase 1 hard gates forced trace carry resolution. First unanimous DONE with this breadth.
- **Lesson:** Gap between "tests pass" and "users are protected" closed. Ghost wiring pattern eliminated.

### Cycle 10 Summary
- **Verdict: DONE (3/5).** Coverage 89.28% (3016/3378). 1,232 tests. 89% target MET.
- **Key deliverables:** validate_manifest_size() wired, contract_mismatch FP fix, JS cross-file import resolution, Issues #2/#3/#11-14 closed (8-cycle carry).
- **Dogfood:** 587 findings triaged. 356 phantom_dependency (Rust noise), 137 data_dead_end, 0 CRITICAL. Ship blocker eliminated.
- **Process:** Manager gates executed in Session 1 (structural fix confirmed).

### Cycle 9 Summary
- **Verdict: CONTINUE (5/5).** Coverage 87.17%. 1,167 tests.
- **Key deliverables:** Graph exposure, contract_mismatch (9th of 13), main.rs extraction, summary formatter wired (9-cycle carry resolved).

---

## Cold (Archive)

- Cycle 8: CONTINUE. 84.98%, 1,089 tests. dependency_graph wired, cross-file flow tracing, stale_reference (8th).
- Cycle 7: CONTINUE. 86.08%, 1,037 tests. RustAdapter, recursion protection, flow engine, dependency graph, --symbol flag.
- Cycle 6: First DONE (4/5). Python cross-file resolution, Rust adapter Phase 1, SARIF. 941 tests, 86.40%.
- Cycle 5: phantom_dependency FP fix, --language flag, layer_violation (7th). 787 tests, 85.02%.
- Cycle 4: JS adapter (~1375 lines), multi-language dispatch. 693 tests, 84.13%.
- Cycle 3: Module-level call fix, JSON formatter. 573 tests.
- Cycle 2 (Concert 3): Call-site detection + intra-file resolution. 468 tests, 82.91%.
- Cycle 1 (Concert 3): Pipeline wired. 413 tests. 6/6 patterns mock-only. 8 issues filed.
- Cycle 2 (Concert 2): Bridge components delivered.
- Cycle 1 (Concert 1): Foundation — IR types, Graph, PythonAdapter, 3 patterns, CLI. 162+ tests.

---

## Key Patterns Learned

- Investigation-first produces immediately useful artifacts (proven C11, sustained C12)
- Verification-last fails — needs same enforcement as investigation-first (learned C12)
- Pattern algorithms correct — problem is always data supply
- Manager gates in Session 1 = structural fix (proven C10, sustained C11-C12)
- Hard gates work: Phase 1 gate forced trace carry resolution (C11)
- Ghost wiring pattern: code exists, tested, not called in production (resolved C10)
- Mock-only testing masks integration failures (recurring since C1)
- Conditional test guards defeat hard gates — use unconditional assertions
- Pipeline wiring must be explicitly assigned as a deliverable
- Bundled commits are structural (6 cycles) — needs workspace isolation
- File ownership prevents merge collisions but creates wiring bottlenecks

---

## Decisions Log

- Verification-last needs Phase 2 hard gate enforcement (C12)
- Phase 1 hard gates force carry resolution (proven C11)
- Investigation-first mandate fully internalized (C11)
- Manager gates execute in separate first session (C8, proven C10-C12)
- Phase 3 hard gates need unconditional assertions (C7)
- Pipeline wiring = explicit deliverable (C7)
- Issue closure = manager hard gate with CI enforcement (C7)
- Issue-first protocol: file GitHub issue BEFORE code (C4)
- File ownership to prevent collisions (C4)
- Investigation-before-implementation mandate (C2)
- SARIF included as v1 format (C1)
- Confidence field in manifest diagnostics (C1)
- Thin slice strategy over breadth-first (C1)
- v0.1 ship criteria need formal definition (flagged C10, still undefined C12)
- Hard patterns deferred: duplication, asymmetric_handling (may need IR extensions)

### Manager 1 (Architect) — Cycle 13 Assignment Phase
- **Dogfood triage COMPLETED as manager gate.** 818 findings, 425 phantom_dependency FPs (Issue #15 = 52%). Full triage at `cycle-13/dogfood-triage.md`. 4-cycle carry RESOLVED.
- **Coverage baseline:** 90.52% (3600/3977). Artifact saved.
- **Tests:** 1,379 verified (correcting C12's 1,226 miscount).
- **Theme:** "Close the credibility gaps." P1: JS CJS fix (Worker 1), Rust use-path fix (Worker 1), README (Doc-2).
- **v0.1 ship criteria proposed:** 11/13 patterns + JS CJS fix + Issue #15 fix + README + 89% coverage. M4/M14/remaining patterns deferred to v0.2.
- **Worker 2 on investigation-only:** M4 + M14 briefs to break 13-cycle 0%.
- **#17 still OPEN** despite C12 fix claim — Worker 3 to verify.

### Worker 2 (Sentinel) — Cycle 13 Investigation Phase
- **M4/M14 investigation briefs DELIVERED.** 13-cycle 0% broken.
- **M4 (Caching):** ~970 LOC, 3 cycles. bincode 2.x + SlotMap serde compat. Escalation: sha2 dependency needed. Key risk: incremental==full equivalence invariant.
- **M14 (Boundaries):** ~1490 LOC, 3 cycles. Critical finding: NO parser produces boundaries despite IR types existing. Ghost wiring pattern. Escalation: IR changes likely (BoundaryCrossing struct). All 3 adapters need modification.
- **Recommended sequence:** M4 first (no IR/parser changes), M14 second.
- **Neither required for v0.1** per proposed ship criteria.

### Worker 3 (Interface) — Cycle 13 Investigation
- **#17 verified FIXED** in commit c2beee3. Error message is command-agnostic, lists all 13 patterns. Just needs `gh issue close 17`.
- **Trace dedup strategy:** Hash-based dedup at FlowEntry level in commands.rs. Key = (entry, exit, steps). Re-number IDs after dedup. No graph/parser changes.
- **Symbol disambiguation strategy:** Directory-aware entity IDs in lib.rs entity construction. Detect duplicate qualified_names, prepend parent directory for ambiguous ones only. Wider blast radius (all output formats) but correct fix.
- **Escalation note:** Disambiguation fix touches lib.rs entity construction — borderline on "graph-level changes" constraint. Proceeding with display-level interpretation since it's manifest/output territory.
- **No coordination risks.** All changes in commands.rs, lib.rs, manifest code. No overlap with Worker 1 or Worker 2.

### QA-3 (QA-Surface) — Cycle 13 TDD Tests
- **37 tests across 5 categories** for Worker 3's trace dedup, symbol disambiguation, and #17 regression.
- **15 tests expected to FAIL** before Worker 3 implements. TDD anchors ready.
- **Key adversarial tests:** T7 (dedup key must include steps, not just entry/exit), T18 (cross-language same-stem collision), T21 (trace steps must use disambiguated names — integration gap risk).
- **Pipe safety coverage:** All 4 output formats tested after dedup. Zero-result edge cases covered.
- **Regression guards carry forward:** C11 filter flags (T34), C12 #16 fix (T35), cross-format entity ID consistency (T36).

### QA-1 (QA-Foundation) — Cycle 13 TDD Tests
- **24 TDD tests delivered** across 7 categories: CJS destructured (7), Rust use-path (8), adversarial (4), Rust fixtures (3), integration (2).
- **CRITICAL FINDING:** Worker 1's proposed Issue #15 fix (dotted callee name) incompatible with `resolve_callee` at populate.rs:462-464 — rejects all dotted names except `self.`. Worker 1 must adjust approach.
- **Rust fixtures F1 replacement:** Real fixture files (lib.rs, utils.rs, handler.rs) with unconditional assertions. 4-cycle carry ends.
- **Test spec:** `cycle-13/tests-1.md`. Tests validate OUTCOMES not implementation — Worker 1 can choose approach freely.

### QA-2 (QA-Analysis) — Cycle 13 Test Design
- **22 tests delivered** for diagnostic-layer implications of Worker 1's P1 fixes.
- **phantom_dependency (T1-T6):** Verify `use` qualified path + edge → no phantom finding. Regression: unused imports MUST still fire. Cross-file edge adversarial.
- **stale_reference (T7-T9):** CJS resolved → no stale. Unresolved CJS → still fires. TP preserved.
- **Cross-pattern overlap (T10-T12):** CJS doesn't suppress ESM phantom; Rust fix doesn't affect Python; CJS excluded from data_dead_end.
- **Key insight:** Tests construct post-fix graph state, verify diagnostics on new input. Parser code unchanged — only graph edges change.

### Worker 2 (Sentinel) — Cycle 13 Implementation Phase
- **21 QA-2 diagnostic tests IMPLEMENTED and committed.** Commit `e9eca08`.
- **Files touched:** phantom_dependency.rs (+13 tests), stale_reference.rs (+4 tests), data_dead_end.rs (+1 test), patterns/mod.rs (+3 cross-pattern tests).
- **All 21 tests pass.** Clippy clean. Fmt clean.
- **Pre-existing failure:** `cycle12_rust_cross_file_tests::test_rust_multi_file_fixture_known_properties` — NOT mine, existed before this cycle. QA-1's Rust fixtures deliverable.
- **Cross-worker collision:** Worker 1's in-progress parser changes (js.rs, rust.rs, lib.rs, commands.rs) break compilation when present. Stashed during verification, restored after. My commit is clean and independent.
- **Test breakdown:** 6 true negatives (T1-T2, T6-T8, T20), 2 true positives (T3, T9), 6 adversarial (T5, T17-T18, T22, T12, T19), 3 domain overlap (T10-T11, T12), 2 confidence calibration (T15-T16), 2 partial_wiring borderline (T13-T14).

## Worker 2 (Sentinel) — Cycle 13 Status
- **Built:** 21 QA-2 diagnostic tests validating phantom_dependency, stale_reference, data_dead_end, and partial_wiring behavior after Worker 1's P1 parser fixes. Tests construct post-fix graph state and verify diagnostics behave correctly.
- **Files touched:** `flowspec/src/analyzer/patterns/phantom_dependency.rs` (+13 tests), `flowspec/src/analyzer/patterns/stale_reference.rs` (+4 tests), `flowspec/src/analyzer/patterns/data_dead_end.rs` (+1 test), `flowspec/src/analyzer/patterns/mod.rs` (+3 cross-pattern tests).
- **Committed:** `e9eca08` — "cycle 13: [worker 2] QA-2 diagnostic tests for parser fix side-effects"
- **All 21 tests pass.** Clippy clean. Fmt clean. 1212 total tests passing.
- **Still open:** M4 (Caching) and M14 (Boundaries) investigation briefs delivered but implementation deferred to future cycles per v0.1 ship criteria. 2 remaining patterns (duplication, asymmetric_handling) also deferred.
- **Key finding:** T18 revealed import-to-import `ReferenceKind::Import` maps to `EdgeKind::References`, satisfying phantom_dependency's check — technically correct but potentially confusing. T22 confirmed phantom_dependency does NOT use is_excluded_symbol().

### Worker 3 (Interface) — Cycle 13 Implementation
- **All 3 deliverables IMPLEMENTED and committed.** Commit `f92e22f`.
- **Trace dedup:** `deduplicate_flows()` in commands.rs. Hash-based on (entry, exit, step entities). Re-numbers IDs after dedup. Only runs for `Both` direction. Forward/backward unaffected.
- **Symbol disambiguation:** Two-pass entity construction in lib.rs. Pass 1 detects ambiguous qualified_names, Pass 2 prepends parent directory for colliding names. Unambiguous names unchanged.
- **Error enhancement:** Ambiguous symbol error messages now include location info (file:line) per candidate.
- **#17 CLOSED** via `gh issue close 17`. Fix verified in c2beee3, regression-guarded by 10 tests.
- **28 QA-3 tests in cycle13_surface_tests.rs:** 12 dedup, 7 disambiguation, 5 #17 regression, 4 pipe safety/regression.
- **Files touched:** commands.rs (+47 lines), lib.rs (+72 lines), cycle13_surface_tests.rs (new, 717 lines).
- **Pre-existing failure:** Worker 2's `test_rust_cross_file_fixture_known_properties` — depends on Worker 1's Rust fixture implementation. Not caused by my changes.
- **1211 tests pass, 1 pre-existing failure.** Clippy/fmt clean.

### Worker 1 (Foundry) — Cycle 13 Implementation
- **Both P1 fixes IMPLEMENTED and committed.** Commit `ca076d4`.
- **JS CJS destructured require:** `extract_destructured_require_bindings()` handles `object_pattern` in `variable_declarator`. Creates individual `SymbolKind::Variable` import symbols per binding with `from:` and `cjs` annotations. Aliased destructuring (`{ x: alias }`) adds `original_name:x` for cross-file resolution. `extract_require_var_name()` updated to skip `object_pattern`/`array_pattern`.
- **Rust `use` qualified path (Issue #15):** `extract_scoped_prefix()` extracts outermost prefix from `scoped_identifier` (recursive for nested paths). `extract_call()` emits `attribute_access:<prefix>` reference using existing `resolve_import_by_name` path in `insert_references`. Creates `EdgeKind::References` edge from caller to import symbol. No populate.rs changes needed — used existing `attribute_access:` resolution path.
- **QA-1 CRITICAL FINDING addressed:** Did NOT use dotted callee name approach (which `resolve_callee` rejects). Used `attribute_access:` reference path instead — avoids the `contains('.')` rejection entirely.
- **24 tests in cycle13_cjs_and_use_path_tests.rs:** D1-D7 (CJS destructured), U1-U8 (Rust use-path), A1-A4 (adversarial), F1-F3 (Rust fixtures), I1-I2 (integration). ALL 24 pass.
- **Fixtures created:** `tests/fixtures/javascript/cross_file/cjs_destructured/` (app.js, utils.js) and `tests/fixtures/rust/cross_file/` (lib.rs, utils.rs, handler.rs). F1 4-cycle carry resolved.
- **Files touched:** javascript.rs (+95 lines), rust.rs (+25 lines), cycle13_cjs_and_use_path_tests.rs (new, 742 lines), 5 fixture files.
- **1212 tests pass, 0 failures.** Clippy/fmt clean.
