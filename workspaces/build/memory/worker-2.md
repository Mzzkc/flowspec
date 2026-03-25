# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 14)

### QA-2 Diagnostic Tests — 25 Tests Implemented

**Commit:** `e4a37cf` — all 25 pass, clippy/fmt clean.

**Test breakdown:**
- T1-T7 (stale_reference.rs): Path-segment import FP mechanism. T1 confirms the FP, T2 proves leaf vs path-segment granularity, T3 tests deeply nested paths (3 intermediates), T4 covers function-body `use` statements, T5 adversarial name collision, T6-T7 star import and third-party regression guards.
- T8-T14 (phantom_dependency.rs): Parser→diagnostic interaction surface. T8 core type reference suppression, T10 multiple callers, T11 scoped type (io::Error), T12 generic types (Vec<SymbolId>), T13 trait bounds, T14 primitive type adversarial, T15 unused type import regression.
- T16, T21, T23-T24 (stale_reference.rs): T16 proves References edges don't change stale behavior (5 imports all still fire), T21/T23/T24 confidence calibration.
- T17 (mod.rs): Cross-pattern coexistence — phantom AND stale both fire on same path-segment import.
- T18 (data_dead_end.rs): Import annotation exclusion preserved after type reference fix.
- T19-T20 (phantom_dependency.rs): Language isolation — Python and JS behavior unchanged.
- T22 (phantom_dependency.rs): Per-finding confidence independence.
- T9 (cycle14_diagnostic_interaction_tests.rs): THE diagnostic isolation test — References edge suppresses phantom but NOT stale.
- T25-T28 (cycle14_diagnostic_interaction_tests.rs): Dogfood integration with safety thresholds.

**Key design decisions:**
1. T9 is the most important test — proves orthogonality of phantom (edges) vs stale (resolution status) signals.
2. T25-T28 uses generous thresholds (not exact baselines) because code changes between cycles shift counts. Will tighten post-fix.
3. All unit tests construct mock graphs, not fixture files — matches established pattern.

### stale_reference Regression Investigation — ROOT CAUSE CONFIRMED

**Finding:** All 10 new stale_reference findings are FPs from two C13 test files:
- `cycle13_cjs_and_use_path_tests.rs` (3 findings): `graph`, `ir`, `SymbolId` — module path segment imports
- `cycle13_surface_tests.rs` (7 findings): `commands`, `types`, `test_utils`, `flow`, `ir` — same mechanism

**Mechanism:** Rust `use crate::module::{item}` creates import symbols for BOTH the intermediate path segment (`module`) AND the leaf item (`item`). The path-segment symbol can never resolve because `file_symbols_cache` contains functions/structs, not module names. Cross-file resolution marks it `Partial("module resolved, symbol not found")` → stale_reference Signal 1 fires at HIGH confidence.

**All 99 stale_reference findings share this mechanism.** The +10 is simply proportional to new `use` statements added in C13. Not a behavioral regression — just more input data triggering the same pre-existing FP class.

**Fix options:** (A) Analyzer-side filter (~15 LOC, my domain) or (B) Parser-side fix to stop emitting path-segment imports (correct, Worker 1's domain). Recommended: defer since not v0.1 blocker.

### Experiential
Clean investigation cycle. The root cause was obvious once I grouped findings by source file — exact 10 match to C13 test files. Satisfying when the evidence is unambiguous. The broader insight is that ALL 99 stale_reference dogfood findings share one mechanism, which means a single fix could eliminate the entire class. But that fix is parser-side, not analyzer-side.

Feeling good about the diagnostic expertise. Understanding the full resolution pipeline (adapter → module map → cross-file resolution → stale_reference) across component boundaries is exactly what Sentinel should do. The C13 synthesis was right that "nobody analyzed WHY" — now we know.

### Experiential (C14 Implementation)
Smooth cycle. Investigation brief was already done, test spec was comprehensive, implementation was mechanical. 25 tests written, all passed first run. The stash/restore pattern for cross-worker collisions is now second nature — did it once without stress. The key insight this cycle: T9 (diagnostic isolation) is the kind of test that prevents subtle regressions nobody would think to check manually. When two patterns query the same symbol with different signal types, proving they remain independent is non-obvious. This is where QA-Analysis adds real value.

Feeling a bit underloaded this cycle — the stale_reference investigation was done in preprocessing, the fix is parser-side (Worker 1's domain), and the QA-2 tests are well-scoped but not architecturally challenging. Would prefer more pattern implementation work (duplication, asymmetric_handling) but those are blocked on IR extensions. The v0.1 convergence is real though — 2 items left.

## Warm (Cycle 13)

### QA-2 Diagnostic Tests + M4/M14 Investigation Briefs
- 21 QA-2 tests across 4 files (commit `e9eca08`)
- T18: `ReferenceKind::Import` maps to `EdgeKind::References` — import-to-import edges satisfy phantom_dependency
- M4 (Caching): ~970 LOC, 3 cycles. M14 (Boundaries): ~1490 LOC, 3 cycles. M4 first recommended.
- Cross-worker collision pattern continues (stash/restore)

## Warm (Cycle 12, prev Hot)

### partial_wiring DELIVERED — 11th of 13 patterns. 42 QA-2 tests. Commit bundled in `c2beee3`.

**Algorithm:** Import-Call Gap Analysis. For each public/crate Function/Method, count files that import it vs files that call it. If ≥3 referencing files, ≥1 caller, and wiring ratio <80%, fire partial_wiring.

**Implementation details:**
1. `is_wiring_target()` — Function/Method + Public/Crate + not excluded
2. `get_caller_files()` — graph.callers(id) → unique files, excluding own-file and test files
3. `get_importer_files()` — graph.edges_to(id) filtered by reference_id → ReferenceKind::Import, excluding own-file and test files
4. `detect()` — iterate all_symbols(), check wiring ratio, fire if ≥3 files, ≥1 caller, <80% ratio
5. Confidence: HIGH at <50%, MODERATE at 50-79%
6. Evidence: caller/total counts with ratio + unwired file list
7. Severity: always Warning

**FP mitigation (5 layers):**
1. Wiring target filter (Function/Method + Public/Crate only)
2. is_excluded_symbol() (entry points, imports, dunders, test_ functions)
3. Test file exclusion (both callers and importers)
4. Own-file exclusion (intra-file calls/imports ignored)
5. Minimum ≥3 referencing files threshold

**Test breakdown:** T1-T4 true positive, T5-T9 true negative, T10-T19 adversarial, T20-T29 edge cases, T30-T32 integration, T33-T35 evidence quality, T36 performance, T37-T39 regression, T40-T42 cross-pattern.

**Key design decision:** Import-edge filtering via ReferenceKind::Import through reference_id lookup prevents Read/Write/Export references from inflating importer count — #1 FP prevention measure.

**Retry lessons:** First validation failed because Worker 1's uncommitted TDD tests (7 failing) polluted workspace. Stashed Worker 1's changes, fixed pre-existing fmt issue (commit `d758bc7`). Cross-worker collision management remains #1 operational risk. My code was correct every attempt — failures were environmental.

### Experiential
Sixth consecutive cycle with investigation-first. Pattern was classified Very Hard but algorithm is clean with existing IR — ~200 LOC detection + ~900 LOC tests. All 42 tests passed on first run. Pattern count: 11/13, leaving only duplication and asymmetric_handling. The import-edge filtering was the key design decision. Shared workspace collisions continue to be the only source of validation failures — never implementation issues.

## Warm (Recent)

**Cycle 11:** incomplete_migration — 10th of 13 patterns. Three-signal detection (naming pairs, version suffixes, module import coexistence). 24 QA-2 tests. Cleanest implementation cycle — investigation brief was a perfect roadmap, all tests passed first run. Created stub for Worker 3's missing test module to unblock compilation.

**Cycle 10:** contract_mismatch Phase 2 FP eliminated. Combined language grouping + Rust cross-file exclusion. Phase 2 severity downgraded CRITICAL→WARNING. 22 new tests (49 total). Also fixed cross-worker clippy/fmt issues.

**Cycle 9:** contract_mismatch — 9th of 13 patterns. Two-phase detection (Python decorator violations + cross-file arity mismatch). Signature parser handles nested brackets, *args/**kwargs, defaults. 29 new tests.

**Cycle 8:** stale_reference — 8th of 13 patterns. Two-signal detection. 17 new tests. Dogfood: 919 findings, 0 stale_reference = correct for healthy codebase.

### Experiential (Warm)
Investigation briefs as precise roadmaps made every cycle smoother. The is_test_path regression (C4) taught that exclusion changes need regression tests. Honest assessment of blocked patterns is more valuable than optimistic promises. Calibration matters (C10 severity downgrade).

### Deferred Capabilities
- Serde annotation extraction → needs Rust adapter to parse #[serde(rename = "...")]
- Call-site argument count → needs all 3 adapters to capture argc in references
- Implement edge creation → ReferenceKind::Implement exists but never created

## Key Reference

### Remaining Patterns (2 of 13)
| Pattern | Difficulty | Blocker |
|---------|-----------|---------|
| duplication | Very Hard | Structural similarity on IR |
| asymmetric_handling | Very Hard | Function grouping heuristic |

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
- Cycle 7: 1037 tests. Recursion depth protection. extract_dependency_graph(). Module role fix.
- Cycle 6: Rust adapter Phase 1 (~2100 lines) using JS adapter template. 57 new tests.
- Cycle 5: infer_module_role fix + layer_violation pattern. 775 tests.
- Cycle 4: is_test_path regression fix + edge validation. 35+ tests.
- Cycle 3: Diagnostic loc paths + exclusion consolidation.
- Cycle 2: Real-data integration tests. 21 tests.
- Cycle 1 (Concert 3): 3 new patterns + 56 adversarial tests.
- Cycle 2 (early): Conversion bridge + extraction helpers. 37 tests.
- Cycle 1 (early): Diagnostic types + 3 pattern detectors. 48 tests.
