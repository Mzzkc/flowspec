# Worker 1 (Foundry) — Memory

## Identity
Infrastructure engineer. Tree-sitter integration, language adapters (Python/JS/Rust), IR design, semantic resolution, persistent graph, cache serialization, incremental analysis. If the foundation is wrong, nothing works.

## Hot (Current Build Cycle — Preprocessor Phase)

### Spec Corpus Analysis

Read all 9 spec files in `.flowspec/spec/`. Here is my domain-specific analysis through the lens of tree-sitter parsing, language adapters, IR, graph, and incremental analysis.

### Key Requirements for My Domain

**1. Parser + Language Adapters (MOSTLY DONE)**
- Python adapter: `parser/python.rs` (2833 lines) — DONE. Includes `__all__`, `TYPE_CHECKING`, type annotation references, relative import support.
- JavaScript/TypeScript adapter: `parser/javascript.rs` (4999 lines) — DONE. ESM, CJS, cross-file, TS preprocessing pipeline (`strip_generics`, `strip_type_annotations`, `strip_implements_clause`), entity dedup.
- Rust adapter: `parser/rust.rs` (2569 lines) — DONE. Intra/cross-file, `use` tree extraction, type references, proximity-based resolution.
- IR types: `parser/ir.rs` (638 lines) — DONE. 11 `SymbolKind` variants, 7 `ReferenceKind` variants, 4 ID types (`SymbolId`, `ScopeId`, `BoundaryId`, `ReferenceId`) via slotmap.
- `LanguageAdapter` trait: `parser/mod.rs` — DONE. `language_name()`, `can_handle()`, `parse_file()`.

**2. Graph Core (DONE, needs extension for cache)**
- `graph/mod.rs` (1268 lines) — Flat slotmap arenas, bidirectional adjacency, file-to-symbol maps. `connected_components()`, `detect_cycles()`, `callees()`, `callers()`, `importers()`.
- `graph/populate.rs` (4171 lines) — 4-phase population (scopes → symbols → references → boundaries). `resolve_cross_file_imports()` with language-specific routing (Python `.`-prefix, JS `./`, Rust `crate::`/`super::`/`self::`).

**3. Persistent Graph + Cache Serialization (NOT STARTED)**
This is the single largest gap in my domain. The spec requires:
- Binary cache at `.flowspec/cache/graph.bin` (architecture.yaml:192-200)
- `file_hashes.json` — SHA256 per source file
- `metadata.json` — version, timestamp, language config
- Serialization via `bincode` (already a dependency, v2)
- `slotmap` has `serde` feature enabled — serialization is possible

The `Graph` struct currently derives `Debug, Clone, Default` but NOT `Serialize/Deserialize` or `Encode/Decode`. This is the first thing to fix. All IR types already derive `Encode, Decode, Serialize, Deserialize` — the graph just needs the same.

**Challenge:** `HashMap<SymbolId, Vec<Edge>>` with slotmap keys. `slotmap` keys implement `Serialize`/`Deserialize` when the `serde` feature is on, but `bincode` v2 uses its own `Encode`/`Decode` traits. Need to verify that the `slotmap` key types work with `bincode` 2.x encode/decode, or implement custom serialization. This is a real risk — slotmap + bincode 2.x compatibility isn't guaranteed out of the box.

**4. Incremental Analysis (NOT STARTED)**
Spec requires (architecture.yaml:56-58, constraints.yaml:86-91):
- First run: parse everything, build full graph, serialize to disk.
- Subsequent runs: load cached graph, diff source files by hash, re-parse only changed files, update the graph.
- Invariant: incremental analysis must produce identical results to full analysis (architecture.yaml:208).
- Performance: incremental under 1 minute for 50K-line codebase (constraints.yaml:89-91).

This requires:
1. File hash computation (SHA256 of each source file)
2. Hash comparison against cached hashes
3. Selective re-parsing (only changed files)
4. Graph update (remove old nodes for changed files, insert new ones)
5. Re-run cross-file resolution on affected file neighborhoods

The `file_symbols` and `file_scopes` maps on `Graph` already support per-file tracking. A `remove_file()` method would remove all symbols/scopes/references for a given file, then `populate_graph()` re-inserts the new version. Cross-file re-resolution is the tricky part — changing file A may affect import resolution in file B.

### Potential Challenges and Risks

1. **bincode 2.x + slotmap compatibility.** The `SlotMap<K, V>` type needs `Encode`/`Decode` impls. slotmap's serde feature gives `Serialize`/`Deserialize` but bincode 2 uses its own traits. May need a wrapper type or use serde-compatible bincode mode. Risk: medium. Mitigation: test early with a small graph.

2. **Incremental cross-file resolution correctness.** If file A changes and defines a new symbol that file B imports, we need to re-resolve file B's imports too, even though B didn't change. This means the "neighborhood" of changed files must be computed. The `file_symbols` map helps, but we also need an import-source-file map (which file does each import resolve from?). This is the hardest correctness challenge.

3. **Graph `remove_file()` implementation.** Need to remove symbols, scopes, references, boundaries, AND all edges pointing to/from removed symbols. The bidirectional adjacency makes this possible but tedious — for every removed symbol, clean up both `outgoing` and `incoming` maps for all connected symbols.

4. **Cache invalidation across Flowspec versions.** metadata.json stores flowspec version — if the IR changes between versions, the cache must be invalidated entirely. Need a cache format version number, not just the Flowspec release version.

5. **tree-sitter-typescript not usable.** Prior memory notes `tree-sitter-typescript` is incompatible with `tree-sitter 0.25`. JS adapter works around this with a preprocessing pipeline (`strip_generics`, `strip_type_annotations`, etc.). This is a known limitation, not a blocker.

6. **Test count: 1870 pass, 6 fail.** All 6 failures are `issues-filed.md` process artifact tests (missing file). Not code issues. The foundation is solid.

### Existing Codebase Map

| File | Lines | Status | Notes |
|------|-------|--------|-------|
| `parser/ir.rs` | 638 | DONE | All IR types with Encode/Decode/Serialize/Deserialize |
| `parser/python.rs` | 2833 | DONE | `__all__`, TYPE_CHECKING, type annotations, relative imports |
| `parser/javascript.rs` | 4999 | DONE | ESM, CJS, TS preprocessing, cross-file |
| `parser/rust.rs` | 2569 | DONE | Intra/cross-file, use trees, type refs |
| `parser/mod.rs` | 60 | DONE | LanguageAdapter trait |
| `graph/mod.rs` | 1268 | DONE | Graph core, needs Serialize/Encode |
| `graph/populate.rs` | 4171 | DONE | Population + cross-file resolution |
| `config/mod.rs` | 417 | DONE | Basic config loading |
| `error.rs` | 134 | DONE | FlowspecError + ManifestError |
| `lib.rs` | 80+ | DONE | Library root, module registration |
| `commands.rs` | ? | DONE | CLI command logic |
| Cargo.toml | 26 | DONE | Dependencies: bincode 2, slotmap (serde), tree-sitter 0.25 |

### Dependencies — What Must Happen First

For cache/incremental to work:
1. **No blockers from other domains.** Cache/incremental is purely my domain.
2. **Graph serialization must work before incremental analysis.** (serial before parallel)
3. **A `remove_file()` method on Graph** is needed before incremental can update.
4. **File hashing infrastructure** (SHA256 of source files) is new code.
5. **The analyze pipeline** (in `commands.rs` / `lib.rs`) needs to branch: if cache exists and is valid → incremental path; else → full analysis path.

### Flags for Other Workers

- **Worker 2 (Sentinel):** 11/13 diagnostic patterns are done. The 2 deferred patterns (duplication, asymmetric_handling) are in `decisions.log` as v1.1. No changes needed from my side for these — they operate on the graph which is stable.
- **Worker 3 (Interface):** The `--incremental / --full` flag on `analyze` command (cli.yaml:57-58) will need CLI wiring once the incremental engine exists. Also `init` command creates `.flowspec/` directory — cache will go in `.flowspec/cache/`.

### Personal Notes / First Impressions

This is a mature codebase. 21 development cycles, 2216+ tests (1870 passing in this workspace, the original has 2216). The parser layer is remarkably complete — three language adapters with cross-file resolution, a clean IR, and the `attribute_access:` piggyback pattern that's been proven across 3 use cases. The graph core is solid but needs serialization support. The ECS-inspired architecture is clean and well-separated.

The biggest gap is cache + incremental, which is entirely in my domain. The spec is clear about what's needed. The risk is in the details — slotmap/bincode compatibility, incremental cross-file resolution correctness, and the `remove_file()` implementation. None of these are architectural risks — they're implementation challenges with known solutions.

The `attribute_access:` piggyback pattern continues to be the most elegant design in this project. It reuses existing graph resolution infrastructure without modifying downstream consumers. If I need to add new reference types for incremental tracking, this pattern is the template.

I feel good about the foundation. The hard work of parsing and semantic resolution is done. What remains is making it fast and persistent — engineering, not research.

### Key Decisions (Stable from Prior Cycles)
- `slotmap` for IDs, `bincode` 2.x for cache, `tree-sitter = "0.25"` pinned
- 1-based line numbers, `HashMap<SymbolId, Vec<Edge>>` adjacency
- BFS for components, iterative DFS for cycles, no petgraph
- SymbolKind::Module filtered from entity list
- Non-self field expressions emit dotted callee names (C11)
- IR types `SymbolKind::Interface`, `SymbolKind::Enum` already existed
- `attribute_access:` piggyback pattern: 3 proven use cases (C20-C21)

### What This Cycle Needs From Me

1. **Immediate:** Write this preprocessor brief (DONE)
2. **Investigation phase:** Verify bincode 2.x + slotmap serialization compatibility. Prototype `Graph` serialization. Design `remove_file()` API.
3. **Implementation phase:** Graph serialization, file hashing, cache read/write, incremental analysis pipeline, `--incremental/--full` flag support.
4. **Testing:** Invariant test: incremental == full on same source. Cache corruption recovery. Version mismatch handling. Performance on realistic projects.

### Codebase Snapshot
- **Tests:** 1870 passing, 6 failing (process artifact, not code)
- **Clippy:** Clean (verified by prior cycles)
- **Fmt:** Clean
- **Dependencies:** bincode 2, slotmap 1.0 (serde), tree-sitter 0.25, chrono, serde, serde_yaml, serde_json, thiserror 2, tracing, glob, ignore

## Concert 4, Cycle 1 — Investigation Phase

### Task: Instance-Attribute Type Resolution

**Investigation complete.** Brief written to `cycle-1/investigation-1.md`.

**Key findings:**
- Tree-sitter-python DOES expose `type` field on `self.attr: Type = value` assignments. No escalation needed.
- `extract_type_annotation_refs` already handles phantom suppression for these annotations (creates `attribute_access:Type` refs).
- The gap is in call resolution: `self.attr.method()` → `resolve_callee` strips `self.` → `_backend.execute` → contains dot → returns default → call edge dropped.
- Fix: new `extract_instance_attr_types()` in python.rs + fallback resolution in `populate_references()` call handler.

**Design decisions (my authority):**
- Reference format: `instance_attr_type:ClassName.attr_name=TypeName`
- v1 scope: simple type annotations only (identifiers, not generics like `Optional[Backend]`)
- Resolution lives in `populate_references()` as a fallback after `resolve_callee` returns default — no changes to `resolve_callee` signature
- Same-file resolution only for v1 (cross-file is stretch goal)

**Files to modify:**
- `parser/python.rs` — new `extract_instance_attr_types()`, call from `parse_file()`
- `graph/populate.rs` — instance-attr fallback in call handler, new `resolve_through_instance_attr()`

**Populate.rs doc changes:** Verified +12 lines at lines 781-797. Correct. Ship with my commit.

### Experiential Notes
Felt right coming back to this codebase. 21 cycles of build memory means I know exactly where things are. The `attribute_access:` piggyback pattern continues to be the right abstraction — this is use case #4 and it fits naturally. The architecture is sound.

The assignment is well-scoped. The manager is right that this is the highest-impact single improvement for real codebases. 40% of orphaned entities from a single resolution gap is a big number. I'm confident in the implementation plan — it follows proven patterns and doesn't require any IR or API changes.

I appreciate being trusted with the heavy technical work again. The preprocessor phase gave me a head start on understanding the problem space, and the investigation phase confirmed my approach is viable. Ready to implement.

## Concert 4, Cycle 1 — Implementation Phase COMPLETE

### What I Built
Instance-attribute type resolution — the `attribute_access:` 4th use case. 6 new functions in python.rs, 1 new function + 1 fallback branch in populate.rs. 24 tests (22 parser + 2 pipeline integration). All pass.

### Key Implementation Details
- `extract_instance_attr_types()` — top-down walk: class → `__init__` → `self.attr: Type` → emit reference
- `walk_for_classes()` — recursive descent, handles nested classes by walking to nearest enclosing class_definition
- `walk_init_body_for_attrs()` — handles if/else/try/except/with/for/while blocks within __init__
- `try_extract_self_attr_type()` — checks left=self.attr, has type field, extracts simple type name
- `extract_simple_type_name()` — returns identifier type names, skips generics and dotted types
- `resolve_through_instance_attr()` — in populate.rs, searches instance_attr_type: refs, matches attr name, finds method on resolved type in same file

### Experiential Notes
Implementation went exactly as planned. The investigation phase paid off — zero surprises from tree-sitter, zero design changes needed. The `attribute_access:` piggyback pattern proved itself for the 4th time. This is the signature architectural pattern of this codebase: reuse existing reference resolution infrastructure without modifying downstream consumers.

The previous attempt had all the code correct but failed to commit. A mechanical error, not a design error. This retry is purely about completing the commit step.

The TDD contract worked well. QA-Foundation's 25 test specs (I implemented 22 parser + 2 integration = 24) were well-designed — they caught real edge cases like annotation-only statements and nested class scoping. The adversarial tests (IADV-1 through IADV-9) exercised boundaries I might not have tested otherwise.

Satisfied with this delivery. The 40% orphaned entity gap is closed for same-file resolution. Cross-file resolution is the natural next step but explicitly out of v1 scope.
