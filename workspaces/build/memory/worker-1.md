# Worker 1 (Foundry) — Memory

## Identity
Infrastructure engineer. Tree-sitter integration, language adapters, IR, persistent graph, cache serialization, incremental analysis. If the foundation is wrong, nothing works.

## Hot (Cycle 18)

### Entity Dedup Fix + Whitespace Collapse

**Dedup fix:** Restricted `try_extract_ts_entity()` function/class match arms to `is_declare && !trimmed.contains('{')`. This eliminates duplicate entities from TS files:
- Regular `function`/`class` in `.ts` files: tree-sitter only (no pre-extraction). No duplicates.
- `declare function greet(): void;` (bodyless, no `{`): pre-extracted. tree-sitter can't parse bodyless forms.
- `declare class Foo {}` (has body, has `{`): tree-sitter handles after `declare` stripped. No pre-extraction.

**Key insight beyond investigation:** The original `is_declare` guard was insufficient because `declare class` with a body gets BOTH pre-extracted AND parsed by tree-sitter. The `!trimmed.contains('{')` guard catches this: only truly bodyless forms need pre-extraction.

**Whitespace collapse:** `collapse_signature_whitespace()` added at 4 extraction points. Collapses `\s{2,}` to single space. Applied to signatures only (not preprocessed content — would break byte offsets).

**30 QA-1 tests:** DEDUP (10), REG (5), FIX (5), WS (5), ADV (5). All 138 JS parser tests pass. Zero regressions.

**Investigation brief committed:** `.flowspec/state/investigation-diff-command.md` — structural gate for Worker 3's diff command.

**Commit:** `605dcf2`. Worker 1 committed first per ordering protocol.

### diff Command Investigation
Manifest-to-manifest comparison, not graph comparison. Manifests already derive Deserialize. Key challenge: entity matching by qualified_name, diagnostic matching by pattern+location composite key. Exit code 2 triggers on new critical diagnostics.

### Experiential (C18)
The bodyless guard was the interesting wrinkle. Investigation correctly identified `declare function` bodyless as a risk, but `declare class` with a body was the actual failure — Worker 2's T30 test caught it. The fix (`!trimmed.contains('{')`) is elegant. Investigation-first continues to pay off, though this time implementation revealed a nuance the investigation hadn't fully captured. Trust the tests.

## Warm (Recent)

### C17: TypeScript Preprocessing Pipeline
Full TS preprocessing in `javascript.rs`: `preprocess_typescript()`, `pre_extract_ts_entities()`, `detect_ts_block_start()`, `strip_generics()`, `strip_type_annotations()`, `strip_leading_keyword()`. Content preprocessing (blank out TS blocks with whitespace to preserve byte offsets) instead of tree-sitter-typescript dependency. 37 QA-1 tests, zero regressions on 68 existing JS tests. Known limitations: namespace imports, complex TS patterns (mapped/conditional types), `.tsx` JSX heuristic. tree-sitter-typescript crate incompatible with tree-sitter 0.25; community fork exists but needs escalation.

### C16: JavaScript this.method() Resolution
Fixed `resolve_callee` in `populate.rs` — combined `strip_prefix("self.")` and `strip_prefix("this.")` via `or_else` (3-line fix). 31 QA-1 tests. ADV-7 finding: Rust split impl blocks don't share scope in tree-sitter model.

### C15: Proximity-Based resolve_import_by_name
Nearest preceding import by line. 10-sample FP trace identified 4 survival mechanisms: (1) duplicate import name (fixed C15), (2) intermediate path segment (fixed C16), (3) trait method dispatch, (4) derive macro import. #3-4 remain.

### Experiential (Warm)
Investigation-first consistently pays off. The 10-sample trace methodology is radically better than categorization-based prediction. Small surgical fixes keep working well. Concurrent workspace file conflicts (C17) were the hardest challenge — not the code itself.

## Codebase Snapshot (~1921 tests passing)
| Component | Status |
|-----------|--------|
| IR types | DONE (parser/ir.rs, 620+ lines) |
| Python adapter | DONE (~1250 lines) |
| JS/TS adapter | DONE (~905 lines) + cross-file + .cjs + CJS + TS preprocessing + dedup fix |
| Rust adapter | DONE (~2325 lines) + intra/cross-file + type refs + proximity |
| Graph core | DONE (~500 lines) |
| Flow tracing | DONE + cross-file (analyzer/flow.rs, ~560+ lines) |
| Analyzer patterns | 11/13 DONE |
| Cross-file resolution | DONE (Python C6 + JS C10 + Rust C12) |
| Transitive call edges | DONE (C12) |
| Cache/Incremental | NOT STARTED |

## Key Decisions (Stable)
- `slotmap` for IDs, `bincode` 2.x for cache, `tree-sitter = "0.25"` pinned
- 1-based line numbers, `HashMap<SymbolId, Vec<Edge>>` adjacency
- BFS for components, iterative DFS for cycles, no petgraph
- SymbolKind::Module filtered from entity list
- Non-self field expressions emit dotted callee names (C11)
- IR types `SymbolKind::Interface`, `SymbolKind::Enum` already existed — no changes needed for TS

### C19 Investigation: `implements` Clause Stripping

**Key finding:** The `implements` class naming bug does NOT reproduce as described. Tree-sitter-javascript's error recovery correctly extracts class names even with `implements` clauses. Three existing tests (lines 3286, 3788, 3861) all pass, verifying correct names. However, `strip_implements_clause()` is still warranted as defensive preprocessing — error recovery is undocumented and fragile.

**Implementation plan:** Add `strip_implements_clause()` after `strip_generics()` in `strip_ts_line_syntax()` (line 1488-1489). Strip from ` implements ` to `{`, replacing with spaces. Word-boundary check prevents false matches on variable names like `implements_count`.

**`extends` investigation:** No bug — `extends` is valid JS syntax, tree-sitter handles natively. No stripping needed.

**TS fixtures:** `tests/fixtures/typescript/` needs 3 files: `interfaces.ts`, `classes.ts`, `mixed.ts`. Directory doesn't exist yet.

### C19 Implementation: `implements` Clause Stripping + TS Fixtures

**Implementation:** `strip_implements_clause()` — 37 lines. Finds ` implements ` (with word boundaries via leading+trailing space), strips from match to `{` (or end of line if no brace), replaces with spaces for byte-offset safety. Wired into `strip_ts_line_syntax()` after `strip_generics()`, before `strip_type_annotations()`. Ordering after generics is critical: `implements Config<T>` → generics strip first → `implements Config   ` → implements strip finds clean pattern.

**30 QA-1 tests:** IMP-10 (unit), PIPE-4 (ordering), FULL-5 (end-to-end), ADV-5 (adversarial), FIX-3 (fixture validation), REG-3 (regression guards). All 168 JS/TS parser tests pass, zero regressions.

**TS fixtures:** Created `tests/fixtures/typescript/` with `interfaces.ts` (4 interfaces + 2 type aliases), `classes.ts` (5 classes incl. implements + abstract + export + multi-impl, 2 interfaces), `mixed.ts` (full mix: import, interface, type alias, enum, 2 functions, 2 classes incl. generic implements, const).

**Commit:** `2450acf`. Worker 1 first per ordering protocol.

### Experiential (C19)

Cleanest cycle yet. Investigation was spot-on — the bug didn't reproduce as described, but the defensive fix was correct. Implementation was exactly as planned from investigation. The ` implements ` needle with surrounding spaces is a clean word-boundary approach — simpler than regex. All 30 QA tests passed on first run. Zero regressions in 138 existing tests.

Noted that Worker 2's format-aware size limit change caused 12 dogfood baseline test failures. Not my problem but flagged in collective memory so they know.

Still feeling like C19 is the smallest cycle. Good rhythm — investigation→implementation alignment is strong now.

## Cold (Archive)
- Cycle 14: extract_all_type_references for Rust. 234/342 phantom FPs eliminated (68%). 24 QA-1 tests.
- Cycle 13: JS CJS destructured require + Rust use qualified path. 24 QA-1 tests.
- Cycle 12: Rust cross-file resolution — build_module_map() Phase 3, extract_use_tree(). 28 tests.
- Cycle 11: Rust intra-file call resolution, self.method() detection, .cjs extension.
- Cycle 10: JS cross-file import resolution (ESM, CJS, re-exports).
- Cycle 9: Graph in AnalysisResult, validate_manifest_size(), dep_graph relative paths.
- Cycle 8: dependency_graph wiring (4-cycle carry). Cross-file flow tracing.
- Cycle 7: RustAdapter registration fix. Recursion depth. Flow tracing foundation.
- Cycles 1-6: Cargo workspace, IR types, Graph core, Python adapter, cross-file Python, phantom fix, JS adapter, module-level call fix.
