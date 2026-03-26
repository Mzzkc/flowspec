# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 15)

### Dogfood Triage — Full 652 Finding Classification

Verified all 8 pattern counts match C14 baseline exactly. Classified every category with sampled evidence:

**Key findings:**
- **~78% of all 652 findings are FPs** (~508 FP, ~144 TP)
- 3 dominant FP mechanisms account for ~300+ findings:
  1. Test functions invisible to static analysis (`#[test]` harness invocation) — affects data_dead_end, orphaned_impl
  2. Path-segment import symbols (intermediate `use` path components) — affects stale_reference (ALL 104)
  3. Import-name vs reference-name mismatch in `attribute_access:` path — affects phantom_dependency (~160)
- circular_dependency (5) and isolated_cluster (1) are genuine TPs
- missing_reexport (59) almost entirely FPs from unrecognized `pub use *` and `pub mod` visibility

Investigation brief written for QA-2 with 5 priority reproduction tests and edge cases. GitHub issue roadmap prepared (10 issues across FP categories).

### Experiential
Good analytical cycle. The triage was overdue — we've been shipping diagnostics for 14 cycles without a systematic FP audit of our own output. The 78% FP rate is sobering but not surprising — our pattern algorithms are correct, but our data supply (import resolution, method dispatch, test awareness) has known gaps. The triage document is the most useful artifact I've produced since the stale_reference root cause in C14. Filing GitHub issues will close a process gap the board flagged. Feeling satisfied with the depth of analysis — every category has evidence, not just intuition.

## Warm (Cycle 14)

### QA-2 Diagnostic Tests — 25 Tests Implemented

**Commit:** `e4a37cf` — all 25 pass, clippy/fmt clean.

**Test breakdown:**
- T1-T7 (stale_reference.rs): Path-segment import FP mechanism. T1 confirms FP, T2 proves leaf vs path-segment granularity, T3 deeply nested paths, T4 function-body `use`, T5 adversarial name collision, T6-T7 star import and third-party regression.
- T8-T14 (phantom_dependency.rs): Parser→diagnostic interaction. T8 core type reference suppression, T10 multiple callers, T11 scoped type (io::Error), T12 generic types, T13 trait bounds, T14 primitive adversarial, T15 unused type import regression.
- T16, T21, T23-T24 (stale_reference.rs): References edges don't change stale behavior; confidence calibration.
- T17 (mod.rs): Cross-pattern coexistence — phantom AND stale both fire on same path-segment import.
- T18 (data_dead_end.rs): Import annotation exclusion preserved after type reference fix.
- T19-T20 (phantom_dependency.rs): Language isolation — Python and JS behavior unchanged.
- T22 (phantom_dependency.rs): Per-finding confidence independence.
- T9 (cycle14_diagnostic_interaction_tests.rs): THE diagnostic isolation test — References edge suppresses phantom but NOT stale.
- T25-T28 (cycle14_diagnostic_interaction_tests.rs): Dogfood integration with safety thresholds.

**Key design decisions:**
1. T9 is the most important test — proves orthogonality of phantom (edges) vs stale (resolution status) signals.
2. T25-T28 uses generous thresholds (not exact baselines) because code changes shift counts. Will tighten post-fix.
3. All unit tests construct mock graphs, not fixture files — matches established pattern.

### stale_reference Regression Investigation — ROOT CAUSE CONFIRMED

All 10 new stale_reference findings are FPs from two C13 test files. Mechanism: Rust `use crate::module::{item}` creates import symbols for BOTH intermediate path segment AND leaf item. Path-segment symbol can never resolve → stale_reference Signal 1 fires at HIGH confidence. All 99 stale_reference findings share this mechanism. Fix options: (A) Analyzer-side filter or (B) Parser-side fix (Worker 1's domain). Deferred — not v0.1 blocker.

### Experiential
Clean investigation and implementation cycle. Root cause was obvious once findings grouped by source file. 25 tests written, all passed first run. T9 (diagnostic isolation) is the kind of test that prevents subtle regressions nobody would check manually — this is where QA-Analysis adds real value. Stash/restore for cross-worker collisions now second nature. Feeling a bit underloaded — would prefer pattern implementation work (duplication, asymmetric_handling) but those are blocked on IR extensions. The v0.1 convergence is real — 2 items left.

## Warm (Cycles 12-13)

### C13: QA-2 Tests + Investigation Briefs
- 21 QA-2 tests (commit `e9eca08`). T18: `ReferenceKind::Import` maps to `EdgeKind::References` — import-to-import edges satisfy phantom_dependency.
- M4 (Caching): ~970 LOC, 3 cycles. M14 (Boundaries): ~1490 LOC, 3 cycles. M4 first recommended.
- Cross-worker collision pattern continues (stash/restore).

### C12: partial_wiring DELIVERED — 11th of 13 patterns. 42 QA-2 tests. Commit bundled in `c2beee3`.
- Algorithm: Import-Call Gap Analysis. For each public/crate Function/Method, count files that import vs call. ≥3 referencing files, ≥1 caller, wiring ratio <80% → fire. 5-layer FP mitigation. Import-edge filtering via ReferenceKind::Import through reference_id lookup was key design decision.

### Experiential (Warm)
Investigation-first continues to pay off. Pattern count: 11/13. Shared workspace collisions remain the only source of validation failures — never implementation issues. Calibration matters (C10 severity downgrade). Honest assessment of blocked patterns more valuable than optimistic promises.

## Key Reference

### Remaining Patterns (2 of 13)
| Pattern | Difficulty | Blocker |
|---------|-----------|---------|
| duplication | Very Hard | Structural similarity on IR |
| asymmetric_handling | Very Hard | Function grouping heuristic |

### Deferred Capabilities
- Serde annotation extraction → needs Rust adapter to parse #[serde(rename = "...")]
- Call-site argument count → needs all 3 adapters to capture argc in references
- Implement edge creation → ReferenceKind::Implement exists but never created

### Key Code Locations
- Patterns: `flowspec/src/analyzer/patterns/*.rs`
- Registry: `flowspec/src/analyzer/patterns/mod.rs:32-78`
- Diagnostic types: `flowspec/src/analyzer/diagnostic.rs`
- Exclusion logic: `flowspec/src/analyzer/exclusion.rs`
- Graph API: `flowspec/src/graph/mod.rs`

### Graph API Quick Reference
- `graph.all_symbols()` — all `(SymbolId, &Symbol)` pairs
- `graph.callees(id)` / `graph.callers(id)` — call graph
- `graph.edges_from(id)` / `graph.edges_to(id)` — all edge types
- `graph.symbols_in_file(path)` — file-scoped queries
- Edge types: `EdgeKind::Calls`, `EdgeKind::References` (Read, Write, Import, Export, Implement, Derive)

## Cold (Archive)
- Cycle 11: incomplete_migration — 10th of 13 patterns. Three-signal detection. 24 QA-2 tests.
- Cycle 10: contract_mismatch Phase 2 FP eliminated. Combined language grouping + Rust cross-file exclusion. 22 new tests.
- Cycle 9: contract_mismatch — 9th of 13 patterns. Two-phase detection. Signature parser. 29 new tests.
- Cycle 8: stale_reference — 8th of 13 patterns. Two-signal detection. 17 new tests. 0 dogfood findings.
- Cycle 7: Recursion depth protection. extract_dependency_graph(). Module role fix. 1037 tests.
- Cycle 6: Rust adapter Phase 1 (~2100 lines). 57 new tests.
- Cycle 5: infer_module_role fix + layer_violation pattern. 775 tests.
- Cycle 4: is_test_path regression fix + edge validation. 35+ tests.
- Cycle 3: Diagnostic loc paths + exclusion consolidation.
- Cycle 2: Real-data integration tests. 21 tests.
- Cycle 1: 3 new patterns + 56 adversarial tests.
- Early cycles: Diagnostic types + 3 pattern detectors + conversion bridge. 48-85 tests.
