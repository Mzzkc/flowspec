# Worker 2 (Sentinel) — Memory

## Identity
Analysis engineer. 13 diagnostic patterns, flow tracing, boundary detection, confidence scoring, evidence generation. I implement `src/analyzer/` — if my analyzers don't work, Flowspec is just a fancy AST printer.

## Hot (Cycle 17)

### Investigation Phase — stale_reference Residual 64

**Finding:** 64 stale_reference findings (was 61 at C16 end, +3 from new test code). Four distinct FP mechanisms identified:

1. **Module-name leaf imports (43/64, 67%):** `use crate::analyzer::patterns::data_dead_end` — `data_dead_end` is a child module, not a symbol in `patterns/mod.rs`. The Rust adapter doesn't extract `mod` declarations as symbols. Fix: check if lookup_name matches a child module file in the module map. Location: `populate.rs:867`.

2. **Macro-generated types (10/64, 16%):** `SymbolId`, `ReferenceId` etc. from `slotmap::new_key_type!`. Tree-sitter can't see macro expansions. Not fixable without macro-specific handling. Deferred.

3. **Re-export resolution (8/64, 12.5%):** `DiagnosticEntry`/`Manifest` imported via `use crate::manifest::*` but defined in `manifest/types.rs`, re-exported through `pub use`. Resolver doesn't follow re-export chains. Deferred (M5 scope).

4. **Test fixture artifacts (3/64, 4.7%):** Intentional stale refs in fixture files. True positives — keep.

**Mixed-language FP investigation:** No cross-language FPs in current dogfood. Fixture dirs are language-segregated. But `populate.rs` module_map doesn't isolate by language — latent bug for mixed-language projects. File issue.

**Fix plan for Phase 2:** Implement Mechanism A fix (module-name child detection) in `populate.rs:867`. Expected: -43 findings, residual ~21.

### Experiential (C17 Investigation)
The investigation was clean and thorough. Four mechanisms instead of the predicted two — macro-generated types and re-export chains are new discoveries. The module-name mechanism is exactly what I predicted in C16, but the root cause is more precise: `mod` declarations aren't symbols, not a name-confusion issue. Feeling confident about the fix — it's surgical like C16's path-segment fix. The attack surface for QA-2 is well-defined.

## Warm (Cycle 16 → moved from Hot)

### Phase 1: Process Deliverables — CLEARED
- 6 GitHub issues filed: #18-#23 covering all FP categories from investigation-2.md
- investigation-2.md committed to `.flowspec/state/` at `074a786`
- 3-cycle process debt resolved. Phase 1 hard gate cleared.

### Phase 2: stale_reference Path-Segment Fix — DELIVERED
**Commit:** `5a7d6f9` — fix + 34 QA-2 tests, all pass, clippy/fmt clean.

**The fix (2 changes in parser/rust.rs):**
1. `is_path_prefix` check at line 698: when `extract_use_tree` recurses into a `scoped_use_list`, the path child (`scoped_identifier`) is the module prefix — NOT a leaf import. Check `node.child_by_field_name("path").is_some_and(|p| p.id() == child.id())` to skip it.
2. `extract_use_path_last_segment` at line 788: handle the recursive case where `use_node` IS the `scoped_use_list` (not a parent of one). This fixes self-import handling (`use crate::module::{self, Item}`).

**Dogfood delta (measured):**
- stale_reference: 117 → 61 (-56). Eliminated true intermediate segments.
- Remaining 61 are module-name LEAF imports and unresolvable type references — a different FP class.
- Total: 620 → 494 (combined with Worker 1's this.method() fix). Then 494 → 441 with all workers.
- phantom: 205 → 135 (Worker 1), orphaned: 53 → 0 (Worker 1), dead_end: 178 → 178 (unchanged).

**34 QA-2 tests across 7 sections:**
- T1-T10: Parser-level TDD (grouped, deep, single, star, aliased, nested, self, multi-stmt, annotation, empty)
- T11-T15: Dogfood regression guards
- T16-T19: C15 flipped guards (FP reproduction → correct behavior)
- T20-T24: Issue verification (one per issue)
- T25-T30: Adversarial (module-as-leaf, enum variants, aliased groups, extern crate, pub use, mixed)
- T31-T34: Cross-pattern interaction (orthogonality, no new phantom, dead_end unaffected, confidence stable)

### Experiential
The fix was small (~15 lines) but the investigation and testing were substantial. The path-segment bug was exactly where I predicted — in `extract_use_tree`'s recursion handling. The self-import fix was a surprise — during recursion, `node` IS the scoped_use_list, but the function was looking for scoped_use_list CHILDREN. Would have been a regression without T7.

Worker 1's concurrent commit changed the dogfood landscape significantly. The 3-cycle process debt is now cleared. 6 issues filed, triage committed. Feeling relief — the hard gates work.

## Warm (Recent)

### C15: QA-2 FP Triage Tests (34 tests) + Dogfood Classification
34 FP reproduction tests across all diagnostic patterns — each constructs minimal graphs that trigger diagnostics on correct code. When fixes land, these become regression guards. Dogfood thresholds use generous safety bounds, not exact counts.

Full 652 finding classification: ~78% FP (~508 FP, ~144 TP). Three dominant FP mechanisms: (1) test functions invisible to static analysis, (2) path-segment import symbols — fixed in C16, (3) import-name vs reference-name mismatch. circular_dependency (5) and isolated_cluster (1) are genuine TPs.

### C14: stale_reference Root Cause + 25 QA-2 Tests
Root cause confirmed: Rust `use crate::module::{item}` creates import symbols for BOTH intermediate path segment AND leaf item. Path-segment symbol can never resolve → stale_reference fires at HIGH. All 99 stale_reference findings share this mechanism. T9 proves orthogonality of phantom vs stale signals.

### Experiential (Warm)
Investigation-first consistently pays off. The 78% FP rate is sobering but not surprising — pattern algorithms are correct, data supply has known gaps. Cross-pattern guards (T32-T34) are valuable for preventing regressions across pattern boundaries. FP reproduction tests document precisely what's broken with named, runnable proofs.

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
- Cycle 12: partial_wiring DELIVERED — 11th of 13 patterns. Import-Call Gap Analysis algorithm.
- Cycle 11: incomplete_migration — 10th of 13 patterns. Three-signal detection.
- Cycle 10: contract_mismatch Phase 2 FP eliminated. Language grouping + Rust cross-file exclusion.
- Cycle 9: contract_mismatch — 9th of 13 patterns. Two-phase detection + signature parser.
- Cycle 8: stale_reference — 8th of 13 patterns. Two-signal detection.
- Cycle 7: Recursion depth protection. extract_dependency_graph(). Module role fix.
- Cycle 6: Rust adapter Phase 1 (~2100 lines).
- Cycles 1-5: Diagnostic types, pattern detectors, integration tests, exclusion consolidation.
