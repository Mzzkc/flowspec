# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 17)

### Investigation — stale_reference Residual 64

**Finding:** 64 stale_reference findings (was 61 at C16 end, +3 from new test code). Four distinct FP mechanisms identified:

1. **Module-name leaf imports (43/64, 67%):** `use crate::analyzer::patterns::data_dead_end` — `data_dead_end` is a child module, not a symbol in `patterns/mod.rs`. Rust adapter doesn't extract `mod` declarations as symbols. Fix: check if lookup_name matches a child module file in the module map. Location: `populate.rs:867`.

2. **Macro-generated types (10/64, 16%):** `SymbolId`, `ReferenceId` etc. from `slotmap::new_key_type!`. Tree-sitter can't see macro expansions. Not fixable without macro-specific handling. Deferred.

3. **Re-export resolution (8/64, 12.5%):** `DiagnosticEntry`/`Manifest` via `use crate::manifest::*` but defined in `manifest/types.rs`, re-exported through `pub use`. Resolver doesn't follow re-export chains. Deferred (M5 scope).

4. **Test fixture artifacts (3/64, 4.7%):** Intentional stale refs in fixture files. True positives — keep.

**Mixed-language FP investigation:** No cross-language FPs in current dogfood. Fixture dirs are language-segregated. But `populate.rs` module_map doesn't isolate by language — latent bug for mixed-language projects. File issue.

**Fix plan for Phase 2:** Implement Mechanism A fix (module-name child detection) in `populate.rs:867`. Expected: -43 findings, residual ~21.

### Experiential (C17)
Investigation was clean and thorough. Four mechanisms instead of predicted two — macro-generated types and re-export chains are new discoveries. The module-name mechanism is exactly what I predicted in C16, but root cause is more precise: `mod` declarations aren't symbols, not name-confusion. Confident about the fix — surgical like C16's path-segment fix.

### C18 Investigation Notes

**Key discovery:** The stash recovery situation is simpler than described. My `is_child_module` fix and the `lib.rs` mod declaration are already on main (committed by Worker 1 in `6920f68`). The ONLY thing missing is committing the untracked test file `cycle17_child_module_tests.rs`. Stash@{0} is redundant with HEAD.

**Dogfood baseline measured:** 495 total findings. data_dead_end=221, phantom_dependency=136, missing_reexport=59, orphaned_impl=53, stale_reference=18, circular_dependency=5, partial_wiring=2, isolated_cluster=1. stale_reference dropped from 64→18 thanks to the child module fix — better than the predicted -43 (got -46).

**Baseline drift explained:** 178→221 is real code growth from C17 additions (TS preprocessing code, init command, test files), NOT a TS duplication artifact. Self-dogfood runs on pure Rust — TS dedup fix won't change these numbers.

### Experiential (C18 Investigation)
Relief. The stash situation that seemed scary is actually fine — my code is already on main, just need to commit one file. The coordination failure in C17 was painful but the actual damage is minimal. Still frustrated that "uncommitted = undelivered" was applied to me when the code WAS there, just the test file got displaced. But the fix is trivial and I can move forward.

Three GitHub issues to file: mixed-language module_map, macro-generated types, re-export resolution. These are my backlog — honest tracking instead of pretending they'll be fixed soon.

## Warm (Recent)

### C16: Process Debt Cleared + stale_reference Path-Segment Fix
Phase 1: 6 GitHub issues filed (#18-#23) covering all FP categories. investigation-2.md committed. 3-cycle process debt resolved. Phase 2: Fix in `parser/rust.rs` — `is_path_prefix` check at line 698 (skip module prefix in `extract_use_tree` recursion) + `extract_use_path_last_segment` at line 788 (handle recursive case where node IS the scoped_use_list). 34 QA-2 tests. Dogfood: stale_reference 117→61 (-56). Total 620→441 combined with all workers.

### C15: QA-2 FP Triage Tests + Dogfood Classification
34 FP reproduction tests across all diagnostic patterns. Full 652 finding classification: ~78% FP (~508 FP, ~144 TP). Three dominant FP mechanisms: test functions invisible to static analysis, path-segment import symbols (fixed C16), import-name vs reference-name mismatch.

### C14: stale_reference Root Cause + 25 QA-2 Tests
Root cause confirmed: Rust `use crate::module::{item}` creates import symbols for BOTH intermediate path segment AND leaf item. Path-segment symbol can never resolve → stale_reference fires at HIGH.

### Experiential (Warm)
Investigation-first consistently pays off. The 78% FP rate is sobering but known — pattern algorithms correct, data supply has known gaps. Cross-pattern guards valuable for preventing regressions. Small surgical fixes keep working. The self-import fix in C16 was a surprise — would have been a regression without T7.

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
- Cycle 13: QA-2 tests + investigation briefs. ReferenceKind::Import → EdgeKind::References mapping.
- Cycle 12: partial_wiring DELIVERED — 11th of 13 patterns. Import-Call Gap Analysis.
- Cycle 11: incomplete_migration — 10th of 13 patterns. Three-signal detection.
- Cycle 10: contract_mismatch Phase 2 FP eliminated. Language grouping + Rust cross-file exclusion.
- Cycle 9: contract_mismatch — 9th of 13 patterns. Two-phase detection + signature parser.
- Cycle 8: stale_reference — 8th of 13 patterns. Two-signal detection.
- Cycles 1-7: Diagnostic types, pattern detectors, integration tests, exclusion consolidation, Rust adapter Phase 1, recursion depth, module role fix.
