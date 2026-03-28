# QA-2 (QA-Analysis) Memory

## Cycle 1 — Test Design Phase

**What I did:** Wrote 30 tests across 9 categories for Worker 2 (Sentinel)'s deliverables: duplication pattern (8 tests), asymmetric handling pattern (7 tests), pattern registration (2), boundary wiring (3), flow type tracking (2), key flows (2), exit points (3), confidence calibration (2), integration (1). 18 of 30 are adversarial.

**Key learning — boundary pipeline is dry:** No language adapter produces Boundary objects. The `boundaries` SlotMap in Graph is always empty. Worker 2 is deriving module boundaries from cross-file edges — my tests validate that approach, not the original IR-based pipeline.

**Key learning — test_utils patterns:** All existing pattern tests use `make_symbol()`, `make_import()`, `add_ref()` from `test_utils.rs` to build graphs manually. The `Symbol.signature` field is `Option<String>` defaulting to `None` — need to set it explicitly for arity-based tests.

**What I feel:** Confident about the duplication tests — the Jaccard threshold boundary tests are mathematically precise. Less confident about asymmetric handling tests — the grouping heuristic is inherently fuzzy and my adversarial tests may need recalibration once I see the actual implementation's grouping logic.

**What concerns me:** The self-recursive exclusion (test 1.7) is a subtle edge case. If `graph.callees(id)` returns `id` itself for recursive functions, the implementation MUST filter it out, or Jaccard computations will be systematically wrong for recursive functions.

**Files I wrote:**
- `workspaces/build/cycle-1/tests-2.md` — full test specification

**Next cycle priorities:** Verify tests against actual implementation. Convert test specs to runnable Rust `#[test]` code if needed. Recalibrate adversarial tests based on actual false positive/negative rates.
