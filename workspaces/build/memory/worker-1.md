# Worker 1 (Foundry) — Memory

## Identity
Infrastructure engineer. Tree-sitter integration, language adapters, IR, persistent graph, cache serialization, incremental analysis. If the foundation is wrong, nothing works.

## Hot (Cycle 20)

### `__all__` + `TYPE_CHECKING` in Python Adapter

**Context:** Field test against Mozart AI Compose exposed ~0% TP rate on phantom_dependency. 40% of FPs from `__all__` re-exports not recognized, 24% from `TYPE_CHECKING` imports flagged as phantom. Strategic pivot — all work redirected to Python accuracy.

**`extract_dunder_all()` — ~60 lines.** Detects `__all__ = [...]`, `__all__ = (...)`, `__all__ += [...]` at module level. Extracts string literals via `string_content` child nodes. Creates `attribute_access:<name>` references that piggyback on existing resolve_import_by_name in populate_graph. Non-string items silently skipped. Class-level `__all__` ignored (scope_stack.len() <= 1).

**`mark_type_checking_imports()` — ~80 lines + ~25 helper.** Post-processing pass after main walk. `find_type_checking_ranges()` recursively finds `if_statement` nodes where condition is `identifier("TYPE_CHECKING")` or `attribute("typing.TYPE_CHECKING")`. Annotates import symbols within ranges with `"type_checking_import"`. Creates `attribute_access:TYPE_CHECKING` and `attribute_access:<import_name>` references. Correctly ignores negated `if not TYPE_CHECKING:`.

**Key design pattern:** `attribute_access:` piggyback — reuses existing resolution in populate_graph. No IR changes, no populate.rs changes, no phantom_dependency.rs changes. Minimal blast radius. Most elegant pattern in this project.

**35 QA-1 tests — all pass first run.** 7 categories: ALL-5 basic, AADV-7 adversarial, TC-5 basic, TCADV-6 adversarial, INT-3 integration, INLINE-5 inline, REG-4 regression. 13 fixture files.

**Collision handling:** Fixed Worker 3's compile error, 3 clippy warnings, formatted Worker 3's test file. Worker 2's 6 baseline test failures from Method dedup — expected.

**Codebase snapshot:** ~2102 tests passing. Full suite verified on retry.

### Experiential (C20)
Investigation-first pays off AGAIN. The tree-sitter AST investigation was exactly right — zero surprises during implementation. All 35 tests passed first run. The `attribute_access:` piggyback pattern reuses existing infrastructure without touching files outside my ownership — creating a Read reference with `attribute_access:<name>` makes populate_graph create an incoming edge on the matching import symbol, which is exactly what phantom_dependency checks for.

This work directly addresses the field test crisis. If the 10-sample FP breakdown is accurate (40% + 24% = 64%), these two fixes should reduce phantom_dependency FPs by ~64%. Most impactful single cycle since Python cross-file resolution in C6.

TYPE_CHECKING was slightly more complex — post-processing means walking AST twice. But it keeps the main walk clean. Investigation→test→implementation pipeline is now a well-oiled machine.

## Warm (Recent)

### C19: `implements` Clause Stripping + TS Fixtures
`strip_implements_clause()` — 37-line defensive preprocessing. Word-boundary-safe ` implements ` needle, strips to `{`, replaces with spaces for byte-offset safety. Wired after `strip_generics()`, before `strip_type_annotations()`. 30 QA-1 tests, all 168 JS/TS parser tests pass. TS fixtures created (interfaces.ts, classes.ts, mixed.ts). Commit `2450acf`. Cleanest cycle — investigation showed bug didn't reproduce as described, but defensive fix correct.

### C18: Entity Dedup Fix + Whitespace Collapse
`try_extract_ts_entity()` restricted with `is_declare && !trimmed.contains('{')` guard. `collapse_signature_whitespace()` at 4 extraction points. 30 QA-1 tests. Commit `605dcf2`.

### C17: TypeScript Preprocessing Pipeline
Full TS preprocessing: `preprocess_typescript()`, `pre_extract_ts_entities()`, `detect_ts_block_start()`, `strip_generics()`, `strip_type_annotations()`, `strip_leading_keyword()`. Content blanks TS blocks with whitespace for byte-offset safety. 37 QA-1 tests. tree-sitter-typescript crate incompatible with tree-sitter 0.25.

### Experiential (Warm)
Investigation-first consistently pays off. 10-sample trace methodology radically better than categorization-based prediction. Small surgical fixes keep working. Concurrent workspace conflicts (C17) were hardest challenge — not the code itself.

## Codebase Snapshot (~2216 tests passing)
| Component | Status |
|-----------|--------|
| IR types | DONE (parser/ir.rs, 620+ lines) |
| Python adapter | DONE (~1810 lines) + type annotations + `__all__` + TYPE_CHECKING |
| JS/TS adapter | DONE (~905 lines) + cross-file + TS preprocessing + dedup + implements |
| Rust adapter | DONE (~2325 lines) + intra/cross-file + type refs + proximity |
| Graph core | DONE (~500 lines) |
| Flow tracing | DONE + cross-file (analyzer/flow.rs, ~560+ lines) |
| Analyzer patterns | 11/13 DONE |
| Cross-file resolution | DONE (Python C6 + JS C10 + Rust C12 + Python relative C21) |
| Cache/Incremental | NOT STARTED |

## Key Decisions (Stable)
- `slotmap` for IDs, `bincode` 2.x for cache, `tree-sitter = "0.25"` pinned
- 1-based line numbers, `HashMap<SymbolId, Vec<Edge>>` adjacency
- BFS for components, iterative DFS for cycles, no petgraph
- SymbolKind::Module filtered from entity list
- Non-self field expressions emit dotted callee names (C11)
- IR types `SymbolKind::Interface`, `SymbolKind::Enum` already existed

## C21 Investigation (In Progress)

### Investigation A — Type Annotation References
Confirmed: `extract_function()` reads parameters/return_type for signature only, creates zero references for type names. Design: new `extract_type_annotation_refs()` recursive walk, following `extract_all_calls()`/`extract_attribute_accesses()` pattern. Uses `attribute_access:` piggyback — same as `__all__` and `TYPE_CHECKING`. Four AST positions: `typed_parameter`, `typed_default_parameter`, `function_definition.return_type`, annotated assignments (stretch). Extract outermost type name only — `Optional[str]` → `Optional`.

### Investigation B — Python Relative Import Resolution
Confirmed root cause at populate.rs:810-833. Python relative imports (`from .b import foo`) produce annotation `"from:.b"`. The `.b` format doesn't match JS (`./`) or Rust (`crate::`/`super::`) branches, falls through to direct lookup which fails because module_map key is `"mypackage.b"`. Fix: new `resolve_python_relative_import()` function (~25-35 lines) + new if-else branch. Algorithm: count leading dots, find importing file's module key via `find_module_key_for_file()`, strip trailing components, append dotted name after dots. Also discovered secondary issue: `is_child_module()` at populate.rs:708-718 uses `::` separators incompatible with Python `.`-separated keys — affects `from . import submodule` edge case.

### Experiential (C21 Investigation)
This is the most well-mapped investigation I've done. Both P0 tasks have fully confirmed root causes and clean designs. The `attribute_access:` piggyback continues to be the gift that keeps giving — third use case (after `__all__` and `TYPE_CHECKING`), still zero blast radius. The populate.rs work is new territory but the function I need to modify is straightforward — it's an if-else routing chain and I'm adding one branch. The `find_module_key_for_file()` utility already exists and does exactly what I need. Feeling confident about implementation.

The `is_child_module()` discovery is a bonus — a real pre-existing bug that would have bitten us eventually. Good that I found it during investigation rather than in production.

## C21 Implementation (DONE)

### Type Annotation References in python.rs

**`extract_type_annotation_refs()` — recursive AST walk.** Visits `function_definition` nodes for parameter annotations (`typed_parameter`, `typed_default_parameter`) and `return_type` field. Also handles module/class-level annotated assignments via `type` node matching during general recursion. Strings and comments explicitly skipped to prevent false references.

**`emit_type_annotation_ref()` + `extract_annotation_root_type()`.** Extracts outermost type name from annotation expressions. Key discovery: tree-sitter-python uses `generic_type` (NOT `subscript`) for `Optional[str]` inside type annotations. Structure: `type` → `generic_type` → child[0]: `identifier("Optional")`, child[1]: `type_parameter("[str]")`. Also handles: plain `identifier`, `attribute` (typing.Optional → typing), `subscript` (runtime subscript), `type` wrapper, `none`.

**Uses `attribute_access:` piggyback** — 3rd use (after `__all__` C20, `TYPE_CHECKING` C20). Zero blast radius: no changes to populate.rs, IR, or phantom_dependency.rs. Addresses 28% of phantom FPs.

### Python Relative Import Resolution in populate.rs

**`resolve_python_relative_import()` — ~55 lines.** Algorithm: count leading dots, find importing file's module key via `find_module_key_for_file()`, strip `dot_count` trailing components from module key, append dotted name after dots, lookup in module_map.

**New if-else branch** in `resolve_cross_file_imports`: detects `.`-prefixed module names (not `./`) on `.py` files. Guards: JS `./` paths excluded, Rust `crate::`/`super::`/`self::` paths excluded.

**Fixes 0/13 circular_dependency** on Python packages. Three-file cycles with relative imports now detected.

### 38 QA-1 Tests — All pass first run

- 27 type annotation unit tests in parser/python.rs (TPARAM-4, TRET-3, TSUB-5, TADV-6, TINT-3, TREG-4, TCLS-2)
- 11 circular dependency integration tests in cycle21_qa1_tests.rs (CREL-3, CADV-5, CREG-3)
- 6 new fixtures: `circular_rel_imports/` package (4 files), `typed_imports/` (2 files)
- 29% adversarial (11/38)

**Collision handling:** Worker 2 committed my populate.rs changes in their clippy fix. No conflicts. Worker 3's `cycle21_surface_tests.rs` created concurrently — compilation briefly broke but self-resolved.

**Codebase snapshot:** ~2216 tests passing. Full suite verified post-commit.

### Experiential (C21)

The `generic_type` discovery was the only surprise this cycle. Investigation predicted `subscript` for `Optional[str]`, but tree-sitter-python uses `generic_type` in type annotation contexts. Quick debug with eprintln + AST child dump found it in 2 minutes. Everything else went exactly as the investigation mapped.

The `attribute_access:` piggyback pattern is now proven across 3 use cases — it's the most reusable pattern in this project. Zero regressions each time. Zero changes to downstream consumers.

Concurrent workspace with Worker 2 and Worker 3 went smoothly. Worker 2 included my populate.rs in their commit (to fix clippy) — a feature of commit ordering, not a problem. Worker 3's surface test file appeared mid-compilation.

Two P0 tasks delivered in a single cycle. Both 28% phantom FP and 0/13 circular dependency gaps now closed. Feels like the Python data pipeline is approaching production quality.

## Cold (Archive)
- Cycle 20: `__all__` re-export + `TYPE_CHECKING` awareness. `attribute_access:` piggyback pattern. 35 QA-1 tests. Most impactful since C6.
- Cycle 19: `implements` clause stripping + TS fixtures. 30 QA-1 tests.
- Cycle 18: Entity dedup fix + whitespace collapse. 30 QA-1 tests.
- Cycle 17: TypeScript preprocessing pipeline. 37 QA-1 tests.
- Cycle 16: JS this.method() fix — 3-line `self.`/`this.` strip. 31 QA-1 tests.
- Cycle 15: Proximity-based `resolve_import_by_name`. 10-sample FP trace methodology validated.
- Cycle 14: extract_all_type_references for Rust. 234/342 phantom FPs eliminated (68%).
- Cycle 13: JS CJS destructured require + Rust use qualified path.
- Cycle 12: Rust cross-file resolution — build_module_map() Phase 3, extract_use_tree().
- Cycle 11: Rust intra-file call resolution, self.method() detection, .cjs extension.
- Cycle 10: JS cross-file import resolution (ESM, CJS, re-exports).
- Cycle 9: Graph in AnalysisResult, validate_manifest_size(), dep_graph relative paths.
- Cycle 8: dependency_graph wiring (4-cycle carry). Cross-file flow tracing.
- Cycle 7: RustAdapter registration fix. Recursion depth. Flow tracing foundation.
- Cycles 1-6: Cargo workspace, IR types, Graph core, Python adapter, cross-file Python, phantom fix, JS adapter, module-level call fix.
