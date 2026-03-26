# Investigation Brief — Worker 2 (Sentinel): Dogfood FP Triage

**Cycle:** 14-15 (investigation), 16 (committed)
**Scope:** Full classification of all 620 dogfood findings from `flowspec diagnose flowspec/src/`

## Summary

620 total findings across 8 active diagnostic patterns. ~78% are false positives (~484 FP, ~136 TP). Three dominant FP mechanisms account for 300+ findings. All pattern algorithms are correct — the FPs originate from data supply gaps (import resolution, method dispatch, test awareness).

## Finding Breakdown by Pattern

| Pattern | Count | Est. TP | Est. FP | FP Rate | Primary FP Mechanism |
|---------|-------|---------|---------|---------|---------------------|
| phantom_dependency | 205 | ~45 | ~160 | 78% | Import-name vs reference-name mismatch (#20) |
| data_dead_end | 178 | ~83 | ~95 | 53% | Test functions invisible to call graph (#19) |
| stale_reference | 117 | 0 | 117 | 100% | Path-segment intermediate imports (#18) |
| missing_reexport | 59 | ~4 | ~55 | 93% | Glob re-export not recognized (#21) |
| orphaned_impl | 53 | ~13 | ~40 | 75% | Method dispatch invisible (#22) |
| circular_dependency | 5 | 5 | 0 | 0% | All genuine |
| partial_wiring | 2 | 2 | 0 | 0% | All genuine |
| isolated_cluster | 1 | 1 | 0 | 0% | Genuine |

## FP Mechanism Details

### Mechanism 1: Path-Segment Intermediate Imports (stale_reference, 117 FPs)

**Root cause:** `extract_use_tree` in `parser/rust.rs:584-732` emits `add_import_symbol` for intermediate path segments in `use` statements.

For `use crate::parser::ir::{Symbol, Reference}`, the parser emits three imports: `ir` (intermediate), `Symbol` (leaf), `Reference` (leaf). The `ir` symbol has `ResolutionStatus::Partial` because no definition named `ir` exists at that scope. `stale_reference::detect()` fires at HIGH confidence.

**Top intermediate segments:** `ir` (16x), `patterns`, `manifest`, `parser`, `diagnostic`, `exclusion`.

**Fix:** Filter intermediate path segments in `extract_use_tree`. Only emit leaf items. ~10-20 lines. Filed as #18.

### Mechanism 2: Test Functions Invisible to Call Graph (data_dead_end, ~95 FPs)

**Root cause:** `#[test]` functions are invoked by the Rust test harness, which is external to the analyzed source. They have zero callers in the graph.

**Fix:** Mark test functions as entry points or exclude from data_dead_end. Filed as #19.

### Mechanism 3: Import-Name vs Reference-Name Mismatch (phantom_dependency, ~160 FPs)

**Root cause:** Rust code uses imported types via qualified paths (`Type::method()`), which are resolved as `attribute_access:Type::method` references. `phantom_dependency` checks for same-file edges referencing the import symbol, but the edge references the attribute access path, not the bare import name.

**Fix:** Teach phantom_dependency to match import names against attribute_access prefixes. Filed as #20.

### Mechanism 4: Glob Re-export Not Recognized (missing_reexport, ~55 FPs)

**Root cause:** `pub use module::*` creates an import for the module name, not individual symbols. `collect_reexported_names` can't match these against child module public symbols. Filed as #21.

### Mechanism 5: Method Dispatch Invisible (orphaned_impl, ~40 FPs)

**Root cause:** `obj.method()` calls are resolved as attribute accesses, not direct calls. No `EdgeKind::Calls` edge links the call site to the method definition. Filed as #22.

### Mechanism 6: resolve_callee First-Match Bug (phantom_dependency, ~20-30 FPs)

**Root cause:** `resolve_callee` picks the first name match across all files. Functions with common names (detect, format, parse) get wrong edges. Filed as #23.

## Triage Methodology

1. Ran `flowspec diagnose flowspec/src/` to get baseline counts
2. Grouped findings by pattern and sorted by source file
3. For each pattern: sampled 10 findings, traced through detection logic, classified as TP or FP
4. For FPs: identified the root cause mechanism and which component owns the fix
5. Cross-validated by constructing minimal reproduction graphs (34 tests in C15 QA-2)

## Priority Order for Fixes

1. **stale_reference path-segment** (#18) — 100% FP, parser fix, ~10 lines, well-understood
2. **resolve_callee proximity** (#23) — proven algorithm from C15, apply to callee resolution
3. **data_dead_end test awareness** (#19) — parser annotation + analyzer exclusion
4. **phantom_dependency attribute matching** (#20) — analyzer-side, needs careful design
5. **missing_reexport glob handling** (#21) — parser + analyzer coordination
6. **orphaned_impl method dispatch** (#22) — depends on method call tracking (Worker 1)
