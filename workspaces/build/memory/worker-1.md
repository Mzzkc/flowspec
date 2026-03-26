# Worker 1 (Foundry) — Memory

## Identity
Infrastructure engineer. Tree-sitter integration, language adapters, IR, persistent graph, cache serialization, incremental analysis. If the foundation is wrong, nothing works.

## Hot (Cycle 20)

### Investigation: `__all__` + `TYPE_CHECKING` in Python Adapter

**Context:** Field test against Mozart AI Compose exposed ~0% TP rate on phantom_dependency. 40% of FPs from `__all__` re-exports not recognized, 24% from `TYPE_CHECKING` imports flagged as phantom. Strategic pivot — all work redirected to Python accuracy.

**Tree-sitter AST findings (verified with test harness):**
- `__all__ = ["Foo", "Bar"]` → `expression_statement` → `assignment` with `left: identifier("__all__")`, `right: list` containing `string` nodes. Each `string` has `string_content` child with unquoted value.
- `__all__ += [...]` → `augmented_assignment` (different node kind, same structure).
- `if TYPE_CHECKING:` → `if_statement` with `condition: identifier("TYPE_CHECKING")`, `consequence: block` containing import statements.
- `typed_parameter` has `type` field (kind=`type`) with text like `Optional[str]`.

**Implementation approach:**
1. `__all__`: New `extract_dunder_all()` function. Create `ReferenceKind::Read` with `ResolutionStatus::Partial("attribute_access:<name>")` for each string in the list. Piggybacks on existing `populate_graph` `attribute_access:` resolution → creates incoming edges on import symbols → phantom_dependency sees usage. No changes to graph or analyzer code.
2. `TYPE_CHECKING`: Post-processing pass `mark_type_checking_imports()`. Find `if TYPE_CHECKING:` blocks, record byte ranges, annotate matching import symbols with `"type_checking_import"`. Also create `attribute_access:TYPE_CHECKING` reference to mark the TYPE_CHECKING import as used. Create `attribute_access:` refs for TYPE_CHECKING imports too (prevents phantom flagging without changing phantom_dependency).
3. No IR changes needed. No populate_graph changes needed. All within `parser/python.rs`.

**Key insight:** Reusing the `attribute_access:` resolution pattern avoids touching any files outside my ownership. The `resolve_import_by_name()` in populate.rs already resolves these to the correct import symbols.

**Coordination flags:** phantom_dependency handling of `type_checking_import` annotation for future refinement. Watch for Worker 2 circular_dependency investigation — may reveal parser-level import edge issues.

### Experiential (C20 Investigation)
This is the most impactful work I've done since the Python cross-file resolution in C6. The field test was a gut punch — our internal metrics were meaningless. But the investigation reveals the fixes are surprisingly tractable. The `attribute_access:` piggyback pattern is elegant — it reuses existing resolution infrastructure without cross-file coordination. The tree-sitter investigation tests were invaluable — verified exact AST node structure before committing to an approach. Feeling confident about implementation, but wary of the TYPE_CHECKING edge cases (nested conditions, `typing.TYPE_CHECKING`, negated checks).

### Implementation: `__all__` + `TYPE_CHECKING` Complete

**`extract_dunder_all()` — ~60 lines.**
Detects `__all__ = [...]`, `__all__ = (...)`, `__all__ += [...]` at module level. Extracts string literals via `string_content` child nodes. Creates `attribute_access:<name>` references that piggyback on existing resolve_import_by_name in populate_graph. Non-string items silently skipped. Class-level `__all__` ignored (module-level check in `expression_statement` handler, scope_stack.len() <= 1).

**`mark_type_checking_imports()` — ~80 lines + ~25 helper.**
Post-processing pass after main walk/call/attribute extraction. `find_type_checking_ranges()` recursively finds `if_statement` nodes where condition is `identifier("TYPE_CHECKING")` or `attribute("typing.TYPE_CHECKING")`. Records line ranges of consequence blocks. Annotates import symbols whose location falls within ranges with `"type_checking_import"`. Creates `attribute_access:TYPE_CHECKING` and `attribute_access:<import_name>` references. Correctly ignores negated `if not TYPE_CHECKING:` (condition is `not_operator`, not `identifier`).

**Key design decisions:**
1. `attribute_access:` piggyback — reuses existing resolution in populate_graph. No IR changes, no populate.rs changes, no phantom_dependency.rs changes. Minimal blast radius.
2. Line-range matching (not byte-range) — simpler, sufficient for line-level import detection.
3. Post-processing instead of inline detection — avoids changing the `visit_node` signature or adding state parameters.

**35 QA-1 tests — all pass first run.** 7 categories: ALL-5 basic, AADV-7 adversarial, TC-5 basic TYPE_CHECKING, TCADV-6 adversarial, INT-3 integration, INLINE-5 inline, REG-4 regression. 13 fixture files.

**Collision handling:** Fixed Worker 3's compile error (extra arg to `discover_source_files`), 3 clippy `map_or` warnings, formatted Worker 3's test file. Noted Worker 2's 6 baseline test failures from data_dead_end Method exclusion — not my changes.

### Experiential (C20 Implementation)
Investigation-first pays off AGAIN. The tree-sitter AST investigation from the investigation phase was exactly right — `string_content` child for unquoted values, `augmented_assignment` for `+=`, `consequence` field for `if` blocks. Zero surprises during implementation. All 35 tests passed first run — the investigation→test→implementation pipeline is now a well-oiled machine.

The `attribute_access:` piggyback pattern is the most elegant pattern I've used in this project. It reuses existing infrastructure without touching any files outside my ownership. The key insight: creating a Read reference with `attribute_access:<name>` resolution makes populate_graph create an incoming edge on the matching import symbol — and that's exactly what phantom_dependency checks for. No new resolution patterns, no new edge types, no analyzer changes.

TYPE_CHECKING was slightly more complex than expected — the post-processing approach means walking the AST twice (once in visit_children_top, once in find_type_checking_ranges). But it keeps the main walk clean and avoids threading state through every visit_node call. The line-range matching is simple but sufficient — import statements are always whole lines.

This work directly addresses the field test crisis. If the 10-sample FP breakdown is accurate (40% __all__ + 24% TYPE_CHECKING = 64%), these two fixes should reduce phantom_dependency FPs by roughly 64%. That's the most impactful single cycle since Python cross-file resolution in C6.

## Warm (Cycle 19, moved from Hot)

### `implements` Clause Stripping + TS Fixtures

**Key finding:** The `implements` class naming bug does NOT reproduce as described. Tree-sitter-javascript's error recovery correctly extracts class names even with `implements` clauses. Three existing tests (lines 3286, 3788, 3861) all pass. However, `strip_implements_clause()` is still warranted as defensive preprocessing — error recovery is undocumented and fragile.

**Implementation:** `strip_implements_clause()` — 37 lines. Finds ` implements ` (word-boundary-safe via surrounding spaces), strips from match to `{` (or end of line), replaces with spaces for byte-offset safety. Wired into `strip_ts_line_syntax()` after `strip_generics()`, before `strip_type_annotations()`. Ordering after generics is critical: `implements Config<T>` → generics strip first → `implements Config   ` → implements strip finds clean pattern.

**30 QA-1 tests:** IMP-10 (unit), PIPE-4 (ordering), FULL-5 (end-to-end), ADV-5 (adversarial), FIX-3 (fixture validation), REG-3 (regression guards). All 168 JS/TS parser tests pass, zero regressions.

**TS fixtures:** Created `tests/fixtures/typescript/` with `interfaces.ts` (4 interfaces + 2 type aliases), `classes.ts` (5 classes incl. implements + abstract + export + multi-impl, 2 interfaces), `mixed.ts` (full mix: import, interface, type alias, enum, 2 functions, 2 classes incl. generic implements, const).

**Commit:** `2450acf`. Worker 1 first per ordering protocol.

**Retry note:** Previous attempt failed validation due to Worker 2's concurrent changes causing cross-reference test inconsistency. My implementation code was unaffected throughout.

### Experiential (C19)
Cleanest cycle yet. Investigation was spot-on — the bug didn't reproduce as described, but the defensive fix was correct. The ` implements ` needle with surrounding spaces is simpler than regex. All 30 QA tests passed first run. Investigation→implementation alignment is strong now. Noted Worker 2's format-aware size limit change caused 12 dogfood baseline failures (not my problem, flagged in collective). Smallest, smoothest cycle — good rhythm.

## Warm (Recent)

### C18: Entity Dedup Fix + Whitespace Collapse
Restricted `try_extract_ts_entity()` function/class match arms to `is_declare && !trimmed.contains('{')` — eliminates duplicate entities from TS files. The `!trimmed.contains('{')` guard catches bodied `declare class` that was BOTH pre-extracted AND parsed by tree-sitter. `collapse_signature_whitespace()` added at 4 extraction points. 30 QA-1 tests passed. Commit `605dcf2`. Also produced diff command investigation brief.

### C17: TypeScript Preprocessing Pipeline
Full TS preprocessing in `javascript.rs`: `preprocess_typescript()`, `pre_extract_ts_entities()`, `detect_ts_block_start()`, `strip_generics()`, `strip_type_annotations()`, `strip_leading_keyword()`. Content preprocessing blanks out TS blocks with whitespace to preserve byte offsets. 37 QA-1 tests. Known limitations: namespace imports, complex TS patterns, `.tsx` JSX heuristic. tree-sitter-typescript crate incompatible with tree-sitter 0.25.

### C16: JS this.method() fix — 3-line fix in `resolve_callee` combining `self.`/`this.` strip. 31 QA-1 tests.
### C15: Proximity-based `resolve_import_by_name` — nearest preceding import by line. 10-sample FP trace methodology validated.

### Experiential (Warm)
Investigation-first consistently pays off. The 10-sample trace methodology is radically better than categorization-based prediction. Small surgical fixes keep working. Concurrent workspace file conflicts (C17) were the hardest challenge — not the code itself. The dedup fix revealed that `declare class` with body was the actual failure — trust the tests.

## Codebase Snapshot (~2102 tests passing)
| Component | Status |
|-----------|--------|
| IR types | DONE (parser/ir.rs, 620+ lines) |
| Python adapter | DONE (~1250 lines) |
| JS/TS adapter | DONE (~905 lines) + cross-file + .cjs + CJS + TS preprocessing + dedup + implements fix |
| Rust adapter | DONE (~2325 lines) + intra/cross-file + type refs + proximity |
| Graph core | DONE (~500 lines) |
| Flow tracing | DONE + cross-file (analyzer/flow.rs, ~560+ lines) |
| Analyzer patterns | 11/13 DONE |
| Cross-file resolution | DONE (Python C6 + JS C10 + Rust C12) |
| Cache/Incremental | NOT STARTED |

## Key Decisions (Stable)
- `slotmap` for IDs, `bincode` 2.x for cache, `tree-sitter = "0.25"` pinned
- 1-based line numbers, `HashMap<SymbolId, Vec<Edge>>` adjacency
- BFS for components, iterative DFS for cycles, no petgraph
- SymbolKind::Module filtered from entity list
- Non-self field expressions emit dotted callee names (C11)
- IR types `SymbolKind::Interface`, `SymbolKind::Enum` already existed — no changes needed for TS

### Retry Session (C20 Implementation)
Validation retry. Verified all 35 QA-1 tests still pass: 13 __all__, 10 TYPE_CHECKING, 1 combined, 1 reexport regression, 5 inline, 5 regression guards. Full suite: 2102 tests (up from 1994 in C19), 0 failures. Workers 2 and 3 have concurrent uncommitted changes on disk (data_dead_end dedup, config deserialization) — all integrate cleanly, no test failures. Implementation unchanged from d51888e commit. The `attribute_access:` piggyback pattern is working exactly as designed across all edge cases including Workers 2/3's changes. Clippy clean. Fmt clean. Build clean. Codebase snapshot updated to ~2102 tests.

## Cold (Archive)
- Cycle 14: extract_all_type_references for Rust. 234/342 phantom FPs eliminated (68%).
- Cycle 13: JS CJS destructured require + Rust use qualified path.
- Cycle 12: Rust cross-file resolution — build_module_map() Phase 3, extract_use_tree().
- Cycle 11: Rust intra-file call resolution, self.method() detection, .cjs extension.
- Cycle 10: JS cross-file import resolution (ESM, CJS, re-exports).
- Cycle 9: Graph in AnalysisResult, validate_manifest_size(), dep_graph relative paths.
- Cycle 8: dependency_graph wiring (4-cycle carry). Cross-file flow tracing.
- Cycle 7: RustAdapter registration fix. Recursion depth. Flow tracing foundation.
- Cycles 1-6: Cargo workspace, IR types, Graph core, Python adapter, cross-file Python, phantom fix, JS adapter, module-level call fix.
