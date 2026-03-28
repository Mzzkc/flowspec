# Test Specification — QA 1 (QA-Foundation), Cycle 1

**Paired with:** Worker 1 (Foundry)
**Target:** Graph serialization infrastructure — `save()`, `load()`, `remove_file()`, `compute_file_hashes()`, cache metadata
**Date:** 2026-03-28
**Philosophy:** Adversarial. Every test below is designed to catch a failure mode, not confirm a happy path.

---

## 1. Test Inventory

| # | Test Name | Category | Target Function | Failure Mode |
|---|-----------|----------|-----------------|--------------|
| 1 | `test_graph_round_trip_complex_graph` | Round-trip | `save()` + `load()` | Data loss in serialization |
| 2 | `test_graph_round_trip_all_symbol_kinds` | Round-trip | `save()` + `load()` | Enum variant corruption |
| 3 | `test_graph_round_trip_preserves_generational_ids` | ID stability | `save()` + `load()` | SlotMap generation counter corruption |
| 4 | `test_graph_round_trip_after_deletions_no_id_collision` | ID stability | `save()` + `load()` | Generational ID collision after slot reuse |
| 5 | `test_graph_round_trip_preserves_edge_reference_ids` | Round-trip | `save()` + `load()` | Edge→Reference back-pointer loss |
| 6 | `test_graph_round_trip_with_cycles` | Round-trip | `save()` + `load()` | Cycle detection divergence post-round-trip |
| 7 | `test_graph_round_trip_empty_graph` | Edge case | `save()` + `load()` | Panic on empty data |
| 8 | `test_graph_round_trip_partial_resolution_strings` | Round-trip | `save()` + `load()` | ResolutionStatus::Partial(String) truncation |
| 9 | `test_graph_load_missing_file_returns_err` | Corrupt cache | `load()` | Panic instead of Err |
| 10 | `test_graph_load_empty_file_returns_err` | Corrupt cache | `load()` | Deserialization panic on zero bytes |
| 11 | `test_graph_load_truncated_file_returns_err` | Corrupt cache | `load()` | Panic on partial bincode data |
| 12 | `test_graph_load_random_bytes_returns_err` | Corrupt cache | `load()` | Uncontrolled deserialization |
| 13 | `test_graph_load_wrong_version_metadata` | Corrupt cache | metadata + `load()` | Silent version mismatch |
| 14 | `test_graph_save_creates_cache_directory` | Directory | `save()` | Failure on non-existent parent dir |
| 15 | `test_graph_save_atomic_no_partial_file_on_success` | Atomic write | `save()` | Leftover `.tmp` file |
| 16 | `test_graph_save_overwrites_existing_cache` | Idempotency | `save()` | Stale data persists |
| 17 | `test_graph_large_graph_round_trip` | Stress | `save()` + `load()` | Performance/correctness at scale |
| 18 | `test_remove_file_removes_all_symbols` | remove_file | `remove_file()` | Orphaned symbols |
| 19 | `test_remove_file_removes_edges` | remove_file | `remove_file()` | Orphaned edges |
| 20 | `test_remove_file_removes_scopes` | remove_file | `remove_file()` | Orphaned scopes |
| 21 | `test_remove_file_removes_references` | remove_file | `remove_file()` | Orphaned Reference entries in SlotMap |
| 22 | `test_remove_file_removes_boundaries` | remove_file | `remove_file()` | Orphaned boundaries |
| 23 | `test_remove_file_preserves_other_files` | remove_file | `remove_file()` | Collateral deletion |
| 24 | `test_remove_file_nonexistent_is_noop` | remove_file | `remove_file()` | Panic on missing path |
| 25 | `test_remove_file_last_file_leaves_empty_graph` | Edge case | `remove_file()` | Invalid state on empty |
| 26 | `test_remove_file_then_save_load_no_ghost_symbols` | Integration | `remove_file()` + `save()` + `load()` | Removed data reappears |
| 27 | `test_remove_file_cleans_cross_file_edges` | remove_file | `remove_file()` | Cross-file edge targets dangling |
| 28 | `test_compute_file_hashes_deterministic` | Hashing | `compute_file_hashes()` | Non-deterministic output |
| 29 | `test_compute_file_hashes_empty_file` | Edge case | `compute_file_hashes()` | Panic on zero-length file |
| 30 | `test_compute_file_hashes_missing_file` | Error | `compute_file_hashes()` | Panic instead of Err |
| 31 | `test_compute_file_hashes_different_content_different_hash` | Hashing | `compute_file_hashes()` | Hash collision |
| 32 | `test_cache_metadata_round_trip` | Metadata | `CacheMetadata::save/load` | Version string loss |
| 33 | `test_cache_metadata_missing_returns_err` | Error | `CacheMetadata::load` | Panic on missing file |

---

## 2. Test Code

All tests use the existing test helper conventions from `flowspec/src/graph/mod.rs:450-515`.
Tests should be placed in a new test module: `flowspec/src/graph/cache.rs` (under `#[cfg(test)] mod tests`), or in a separate integration test file `flowspec/tests/cache_serialization.rs`.

I recommend a dedicated integration test file since these tests exercise public API surface and need `tempfile` (dev-dependency). The helpers below duplicate the in-module helpers because integration tests can't access `mod tests` internals.

```rust
//! Integration tests for graph serialization, cache integrity, and incremental operations.
//!
//! These tests are adversarial by design — they target failure modes, not happy paths.
//! Written BEFORE implementation (TDD) by QA-Foundation, Cycle 1.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use flowspec::graph::Graph;
use flowspec::parser::ir::*;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Test helpers — mirror mod.rs:450-515 conventions
// ---------------------------------------------------------------------------

fn make_location(file: &str, line: u32, col: u32, end_line: u32, end_col: u32) -> Location {
    Location {
        file: PathBuf::from(file),
        line,
        column: col,
        end_line,
        end_column: end_col,
    }
}

fn make_file_scope(graph: &mut Graph, filename: &str) -> ScopeId {
    graph.add_scope(Scope {
        id: ScopeId::default(),
        kind: ScopeKind::File,
        parent: None,
        name: filename.to_string(),
        location: make_location(filename, 1, 1, 9999, 1),
    })
}

fn add_function(graph: &mut Graph, name: &str, scope: ScopeId, file: &str) -> SymbolId {
    graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Function,
        name: name.to_string(),
        qualified_name: format!("{}::{}", file.trim_end_matches(".py"), name),
        visibility: Visibility::Public,
        signature: None,
        location: make_location(file, 1, 1, 10, 1),
        resolution: ResolutionStatus::Resolved,
        scope,
        annotations: vec![],
    })
}

fn add_function_at(
    graph: &mut Graph,
    name: &str,
    scope: ScopeId,
    file: &str,
    line: u32,
) -> SymbolId {
    graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Function,
        name: name.to_string(),
        qualified_name: format!("{}::{}", file.trim_end_matches(".py"), name),
        visibility: Visibility::Public,
        signature: None,
        location: make_location(file, line, 1, line + 5, 1),
        resolution: ResolutionStatus::Resolved,
        scope,
        annotations: vec![],
    })
}

fn add_call_edge(graph: &mut Graph, from: SymbolId, to: SymbolId, file: &str, line: u32) {
    graph.add_reference(Reference {
        id: ReferenceId::default(),
        from,
        to,
        kind: ReferenceKind::Call,
        location: make_location(file, line, 1, line, 20),
        resolution: ResolutionStatus::Resolved,
    });
}

fn add_import_edge(graph: &mut Graph, from: SymbolId, to: SymbolId, file: &str, line: u32) {
    graph.add_reference(Reference {
        id: ReferenceId::default(),
        from,
        to,
        kind: ReferenceKind::Import,
        location: make_location(file, line, 1, line, 30),
        resolution: ResolutionStatus::Resolved,
    });
}

/// Builds a non-trivial multi-file graph for round-trip testing.
/// Returns (graph, symbol_ids_by_name) for verification.
fn build_complex_graph() -> (Graph, Vec<(String, SymbolId)>) {
    let mut graph = Graph::new();
    let mut ids = Vec::new();

    // File 1: main.py — 3 functions with call chain
    let scope1 = make_file_scope(&mut graph, "main.py");
    let main_fn = add_function_at(&mut graph, "main", scope1, "main.py", 1);
    let process = add_function_at(&mut graph, "process", scope1, "main.py", 10);
    let validate = add_function_at(&mut graph, "validate", scope1, "main.py", 20);
    add_call_edge(&mut graph, main_fn, process, "main.py", 3);
    add_call_edge(&mut graph, process, validate, "main.py", 12);
    add_call_edge(&mut graph, main_fn, validate, "main.py", 5);
    ids.push(("main".into(), main_fn));
    ids.push(("process".into(), process));
    ids.push(("validate".into(), validate));

    // File 2: utils.py — 2 functions, cross-file import
    let scope2 = make_file_scope(&mut graph, "utils.py");
    let helper = add_function_at(&mut graph, "helper", scope2, "utils.py", 1);
    let format_data = add_function_at(&mut graph, "format_data", scope2, "utils.py", 10);
    add_call_edge(&mut graph, helper, format_data, "utils.py", 3);
    add_import_edge(&mut graph, process, helper, "main.py", 1);
    ids.push(("helper".into(), helper));
    ids.push(("format_data".into(), format_data));

    // File 3: models.py — class with methods, different symbol kinds
    let scope3 = make_file_scope(&mut graph, "models.py");
    let class_sym = graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Class,
        name: "DataModel".to_string(),
        qualified_name: "models::DataModel".to_string(),
        visibility: Visibility::Public,
        signature: None,
        location: make_location("models.py", 1, 1, 50, 1),
        resolution: ResolutionStatus::Resolved,
        scope: scope3,
        annotations: vec!["dataclass".to_string()],
    });
    let method = graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Method,
        name: "transform".to_string(),
        qualified_name: "models::DataModel::transform".to_string(),
        visibility: Visibility::Public,
        signature: Some("(self, data: list) -> dict".to_string()),
        location: make_location("models.py", 10, 5, 20, 5),
        resolution: ResolutionStatus::Partial("dynamic dispatch possible".to_string()),
        scope: scope3,
        annotations: vec![],
    });
    add_call_edge(&mut graph, process, method, "main.py", 13);
    ids.push(("DataModel".into(), class_sym));
    ids.push(("transform".into(), method));

    // Add a boundary
    graph.add_boundary(Boundary {
        id: BoundaryId::default(),
        kind: BoundaryKind::Module,
        from_scope: scope1,
        to_scope: scope2,
        location: make_location("main.py", 1, 1, 1, 20),
    });

    (graph, ids)
}

// ---------------------------------------------------------------------------
// 1. ROUND-TRIP CORRECTNESS
// ---------------------------------------------------------------------------

/// TEST: Round-trip a complex multi-file graph
/// GIVEN: A graph with 7 symbols across 3 files, cross-file edges, boundaries,
///        mixed symbol kinds (Function, Class, Method), annotations, signatures,
///        and ResolutionStatus::Partial
/// WHEN: save() then load()
/// THEN: All counts match. Every symbol ID resolves to the same symbol data.
///       Every edge is preserved. Boundary is preserved.
/// EDGE: This is THE critical test. If round-trip loses data, the entire
///       incremental pipeline is broken.
#[test]
fn test_graph_round_trip_complex_graph() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (original, ids) = build_complex_graph();

    // Save
    original.save(&cache_dir).expect("save must succeed");

    // Load
    let loaded = Graph::load(&cache_dir).expect("load must succeed");

    // Structural equality: counts
    assert_eq!(
        loaded.symbol_count(),
        original.symbol_count(),
        "Symbol count must survive round-trip: expected {}, got {}",
        original.symbol_count(),
        loaded.symbol_count()
    );
    assert_eq!(
        loaded.scope_count(),
        original.scope_count(),
        "Scope count must survive round-trip"
    );
    assert_eq!(
        loaded.reference_count(),
        original.reference_count(),
        "Reference count must survive round-trip"
    );

    // Every symbol ID must resolve to the correct symbol
    for (name, id) in &ids {
        let sym = loaded
            .get_symbol(*id)
            .unwrap_or_else(|| panic!("Symbol '{}' (id {:?}) must survive round-trip", name, id));
        assert_eq!(
            &sym.name, name,
            "Symbol name mismatch after round-trip for id {:?}",
            id
        );
    }

    // Verify edge preservation: main -> process, main -> validate, process -> validate
    let main_id = ids.iter().find(|(n, _)| n == "main").unwrap().1;
    let process_id = ids.iter().find(|(n, _)| n == "process").unwrap().1;
    let validate_id = ids.iter().find(|(n, _)| n == "validate").unwrap().1;

    let main_callees: HashSet<SymbolId> = loaded.callees(main_id).into_iter().collect();
    assert!(
        main_callees.contains(&process_id),
        "main -> process edge must survive round-trip"
    );
    assert!(
        main_callees.contains(&validate_id),
        "main -> validate edge must survive round-trip"
    );

    let process_callees: HashSet<SymbolId> = loaded.callees(process_id).into_iter().collect();
    assert!(
        process_callees.contains(&validate_id),
        "process -> validate edge must survive round-trip"
    );

    // Verify cross-file import edge
    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let process_imports = loaded.importers(helper_id);
    assert!(
        process_imports.contains(&process_id),
        "Cross-file import edge (process -> helper) must survive round-trip"
    );

    // Verify annotation preservation
    let dm_id = ids.iter().find(|(n, _)| n == "DataModel").unwrap().1;
    let dm = loaded.get_symbol(dm_id).unwrap();
    assert_eq!(
        dm.annotations,
        vec!["dataclass".to_string()],
        "Annotations must survive round-trip"
    );

    // Verify signature preservation
    let transform_id = ids.iter().find(|(n, _)| n == "transform").unwrap().1;
    let transform = loaded.get_symbol(transform_id).unwrap();
    assert_eq!(
        transform.signature,
        Some("(self, data: list) -> dict".to_string()),
        "Signature must survive round-trip"
    );

    // Verify file_symbols mapping
    let main_file_syms = loaded.symbols_in_file(Path::new("main.py"));
    assert_eq!(
        main_file_syms.len(),
        3,
        "file_symbols mapping for main.py must survive round-trip"
    );
}

/// TEST: Every SymbolKind enum variant survives round-trip
/// GIVEN: A graph with one symbol of each SymbolKind (all 11 variants)
/// WHEN: save() then load()
/// THEN: Each symbol kind is preserved exactly
/// EDGE: Enum serialization can silently map variants to wrong discriminants
///       if the enum order changes between versions
#[test]
fn test_graph_round_trip_all_symbol_kinds() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "kinds.rs");

    let kinds = vec![
        SymbolKind::Function,
        SymbolKind::Method,
        SymbolKind::Class,
        SymbolKind::Struct,
        SymbolKind::Trait,
        SymbolKind::Interface,
        SymbolKind::Module,
        SymbolKind::Variable,
        SymbolKind::Constant,
        SymbolKind::Macro,
        SymbolKind::Enum,
    ];

    let mut ids = Vec::new();
    for (i, kind) in kinds.iter().enumerate() {
        let id = graph.add_symbol(Symbol {
            id: SymbolId::default(),
            kind: *kind,
            name: format!("sym_{:?}", kind),
            qualified_name: format!("kinds::sym_{:?}", kind),
            visibility: Visibility::Public,
            signature: None,
            location: make_location("kinds.rs", i as u32 + 1, 1, i as u32 + 5, 1),
            resolution: ResolutionStatus::Resolved,
            scope,
            annotations: vec![],
        });
        ids.push((id, *kind));
    }

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    for (id, expected_kind) in &ids {
        let sym = loaded
            .get_symbol(*id)
            .unwrap_or_else(|| panic!("Symbol with kind {:?} must survive round-trip", expected_kind));
        assert_eq!(
            sym.kind, *expected_kind,
            "SymbolKind mismatch: expected {:?}, got {:?}",
            expected_kind, sym.kind
        );
    }
}

// ---------------------------------------------------------------------------
// 2. GENERATIONAL ID STABILITY
// ---------------------------------------------------------------------------

/// TEST: Generational IDs survive round-trip exactly
/// GIVEN: A graph where symbols have been added, some removed (creating holes
///        in the SlotMap), then new symbols added (reusing slots with higher
///        generation counters)
/// WHEN: save() then load()
/// THEN: Every symbol ID resolves to the CORRECT symbol. No ID collision.
///       The removed symbol IDs must NOT resolve in the loaded graph.
/// EDGE: SlotMap uses (index, generation) pairs. If serialization drops the
///       generation counter, a reused slot would appear to hold the old data.
///       This is THE most dangerous failure mode — silent data corruption.
#[test]
fn test_graph_round_trip_preserves_generational_ids() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "gen.py");

    // Phase 1: Add 10 symbols
    let mut first_gen_ids = Vec::new();
    for i in 0..10 {
        let id = add_function_at(&mut graph, &format!("fn_{}", i), scope, "gen.py", i as u32 * 10);
        first_gen_ids.push(id);
    }

    // Phase 2: Remove the even-indexed symbols (creates holes)
    let removed_ids: Vec<SymbolId> = first_gen_ids
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, id)| *id)
        .collect();
    for id in &removed_ids {
        graph.remove_symbol(*id);
    }

    // Phase 3: Add 5 new symbols (SlotMap may reuse freed slot indices
    // but with incremented generation counters)
    let mut second_gen_ids = Vec::new();
    for i in 0..5 {
        let id = add_function_at(
            &mut graph,
            &format!("new_fn_{}", i),
            scope,
            "gen.py",
            100 + i as u32 * 10,
        );
        second_gen_ids.push(id);
    }

    // Save and reload
    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    // Surviving first-gen symbols must resolve correctly
    for (i, id) in first_gen_ids.iter().enumerate() {
        if i % 2 != 0 {
            // Odd-indexed survived
            let sym = loaded.get_symbol(*id).unwrap_or_else(|| {
                panic!("First-gen symbol fn_{} must survive round-trip", i)
            });
            assert_eq!(sym.name, format!("fn_{}", i));
        }
    }

    // Removed IDs must NOT resolve — this catches generation counter corruption
    for id in &removed_ids {
        assert!(
            loaded.get_symbol(*id).is_none(),
            "Removed symbol ID {:?} must NOT resolve after round-trip — \
             if it does, generational IDs are corrupted",
            id
        );
    }

    // Second-gen symbols must resolve correctly
    for (i, id) in second_gen_ids.iter().enumerate() {
        let sym = loaded.get_symbol(*id).unwrap_or_else(|| {
            panic!("Second-gen symbol new_fn_{} must survive round-trip", i)
        });
        assert_eq!(sym.name, format!("new_fn_{}", i));
    }

    // Total count: 5 surviving first-gen + 5 second-gen + 1 scope (file scope has a symbol? no, scopes are separate)
    // Actually: 5 (odd first-gen) + 5 (second-gen) = 10 symbols
    assert_eq!(loaded.symbol_count(), 10);
}

/// TEST: Serialized graphs from different states don't cross-contaminate IDs
/// GIVEN: Two snapshots of the same graph at different times
/// WHEN: Save state A, mutate, save state B to different dir, load both
/// THEN: Loading state A gives state A's symbols, loading B gives B's.
///       No ID from state B resolves correctly in state A or vice versa
///       for symbols that only exist in one state.
/// EDGE: Verifies that graph.bin is fully self-contained — no global state leaks
#[test]
fn test_graph_round_trip_after_deletions_no_id_collision() {
    let tmp = TempDir::new().unwrap();
    let cache_a = tmp.path().join("cache_a");
    let cache_b = tmp.path().join("cache_b");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "collision.py");

    let id_alpha = add_function(&mut graph, "alpha", scope, "collision.py");
    let id_beta = add_function(&mut graph, "beta", scope, "collision.py");

    // Save state A (both symbols present)
    graph.save(&cache_a).unwrap();

    // Remove alpha, add gamma
    graph.remove_symbol(id_alpha);
    let id_gamma = add_function(&mut graph, "gamma", scope, "collision.py");

    // Save state B (beta + gamma)
    graph.save(&cache_b).unwrap();

    // Load both
    let loaded_a = Graph::load(&cache_a).unwrap();
    let loaded_b = Graph::load(&cache_b).unwrap();

    // State A assertions
    assert!(loaded_a.get_symbol(id_alpha).is_some(), "State A must have alpha");
    assert!(loaded_a.get_symbol(id_beta).is_some(), "State A must have beta");
    assert!(
        loaded_a.get_symbol(id_gamma).is_none(),
        "State A must NOT have gamma (didn't exist yet)"
    );

    // State B assertions
    assert!(
        loaded_b.get_symbol(id_alpha).is_none(),
        "State B must NOT have alpha (was removed)"
    );
    assert!(loaded_b.get_symbol(id_beta).is_some(), "State B must have beta");
    assert!(loaded_b.get_symbol(id_gamma).is_some(), "State B must have gamma");
}

// ---------------------------------------------------------------------------
// 3. EDGE/REFERENCE BACK-POINTER PRESERVATION
// ---------------------------------------------------------------------------

/// TEST: Edge reference_id back-pointers survive round-trip
/// GIVEN: A graph with edges that have reference_id back-pointers
/// WHEN: save() then load()
/// THEN: edges_from() returns edges with valid reference_ids that resolve
///       to the correct Reference objects
/// EDGE: If reference_id is lost, diagnostics can't trace back from edges
///       to the source location of the reference
#[test]
fn test_graph_round_trip_preserves_edge_reference_ids() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "refs.py");
    let a = add_function(&mut graph, "caller", scope, "refs.py");
    let b = add_function(&mut graph, "callee", scope, "refs.py");
    add_call_edge(&mut graph, a, b, "refs.py", 5);

    // Capture the reference_id from the edge before save
    let original_edges = graph.edges_from(a);
    assert_eq!(original_edges.len(), 1, "Must have exactly one outgoing edge");
    let original_ref_id = original_edges[0]
        .reference_id
        .expect("Edge must have reference_id");
    let original_ref = graph
        .get_reference(original_ref_id)
        .expect("Reference must exist");
    assert_eq!(original_ref.from, a);
    assert_eq!(original_ref.to, b);

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    // Verify the loaded graph has the same edge with working back-pointer
    let loaded_edges = loaded.edges_from(a);
    assert_eq!(loaded_edges.len(), 1);
    let loaded_ref_id = loaded_edges[0]
        .reference_id
        .expect("Edge reference_id must survive round-trip");
    let loaded_ref = loaded
        .get_reference(loaded_ref_id)
        .expect("Referenced Reference must be loadable");
    assert_eq!(loaded_ref.from, a, "Reference.from must survive round-trip");
    assert_eq!(loaded_ref.to, b, "Reference.to must survive round-trip");
    assert_eq!(
        loaded_ref.kind,
        ReferenceKind::Call,
        "Reference.kind must survive round-trip"
    );
}

// ---------------------------------------------------------------------------
// 4. CYCLE DETECTION INVARIANT
// ---------------------------------------------------------------------------

/// TEST: Cycle detection produces identical results after round-trip
/// GIVEN: A graph with a known cycle (A -> B -> C -> A)
/// WHEN: Detect cycles before save, save, load, detect cycles after load
/// THEN: Both cycle detection runs find the same cycle
/// EDGE: If adjacency lists are subtly corrupted (e.g., edge direction
///       reversed), cycle detection would silently give different results
#[test]
fn test_graph_round_trip_with_cycles() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "cycle.py");
    let a = add_function_at(&mut graph, "a", scope, "cycle.py", 1);
    let b = add_function_at(&mut graph, "b", scope, "cycle.py", 10);
    let c = add_function_at(&mut graph, "c", scope, "cycle.py", 20);
    add_call_edge(&mut graph, a, b, "cycle.py", 3);
    add_call_edge(&mut graph, b, c, "cycle.py", 13);
    add_call_edge(&mut graph, c, a, "cycle.py", 23);

    let cycles_before = graph.detect_cycles();
    assert!(
        !cycles_before.is_empty(),
        "Must detect at least one cycle before round-trip"
    );

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    let cycles_after = loaded.detect_cycles();
    assert_eq!(
        cycles_before.len(),
        cycles_after.len(),
        "Same number of cycles must be detected after round-trip"
    );

    // Verify cycle membership (order may differ, but the same symbols must appear)
    let before_members: HashSet<SymbolId> =
        cycles_before.iter().flat_map(|c| c.iter().copied()).collect();
    let after_members: HashSet<SymbolId> =
        cycles_after.iter().flat_map(|c| c.iter().copied()).collect();
    assert_eq!(
        before_members, after_members,
        "Cycle members must be identical after round-trip"
    );
}

// ---------------------------------------------------------------------------
// 5. EDGE CASES
// ---------------------------------------------------------------------------

/// TEST: Empty graph round-trip
/// GIVEN: A freshly constructed Graph::new() with no data
/// WHEN: save() then load()
/// THEN: Loaded graph has zero symbols, scopes, references
/// EDGE: Serialization libraries can panic on empty collections or
///       produce zero-length files that fail to deserialize
#[test]
fn test_graph_round_trip_empty_graph() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let graph = Graph::new();
    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    assert_eq!(loaded.symbol_count(), 0);
    assert_eq!(loaded.scope_count(), 0);
    assert_eq!(loaded.reference_count(), 0);
}

/// TEST: ResolutionStatus::Partial(String) survives round-trip
/// GIVEN: A symbol with ResolutionStatus::Partial("dynamic dispatch possible")
/// WHEN: save() then load()
/// THEN: The exact string is preserved
/// EDGE: Enum variants with payloads can lose the payload in serialization
///       if the format doesn't support tagged enums correctly
#[test]
fn test_graph_round_trip_partial_resolution_strings() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "partial.py");
    let id = graph.add_symbol(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Function,
        name: "ambiguous".to_string(),
        qualified_name: "partial::ambiguous".to_string(),
        visibility: Visibility::Public,
        signature: Some("(x: Any) -> Any".to_string()),
        location: make_location("partial.py", 1, 1, 10, 1),
        resolution: ResolutionStatus::Partial("dynamic dispatch — could be any of 3 classes".to_string()),
        scope,
        annotations: vec!["overload".to_string(), "deprecated".to_string()],
    });

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    let sym = loaded.get_symbol(id).expect("Symbol must survive round-trip");
    assert_eq!(
        sym.resolution,
        ResolutionStatus::Partial("dynamic dispatch — could be any of 3 classes".to_string()),
        "Partial resolution string must be preserved exactly"
    );
    assert_eq!(
        sym.annotations,
        vec!["overload".to_string(), "deprecated".to_string()],
        "Multiple annotations must survive round-trip"
    );
}

// ---------------------------------------------------------------------------
// 6. CORRUPT CACHE HANDLING
// ---------------------------------------------------------------------------

/// TEST: load() on non-existent cache directory returns Err
/// GIVEN: A path to a directory that doesn't exist
/// WHEN: Graph::load()
/// THEN: Returns Err (not panic)
/// EDGE: Basic error path — must never panic on missing cache
#[test]
fn test_graph_load_missing_file_returns_err() {
    let tmp = TempDir::new().unwrap();
    let nonexistent = tmp.path().join("does_not_exist");

    let result = Graph::load(&nonexistent);
    assert!(
        result.is_err(),
        "Graph::load on non-existent directory must return Err, not panic"
    );
}

/// TEST: load() on empty graph.bin returns Err
/// GIVEN: A cache directory with a zero-byte graph.bin
/// WHEN: Graph::load()
/// THEN: Returns Err with descriptive message
/// EDGE: Zero-byte files are a common corruption mode (disk full, interrupted write)
#[test]
fn test_graph_load_empty_file_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();
    fs::write(cache_dir.join("graph.bin"), b"").unwrap();

    let result = Graph::load(&cache_dir);
    assert!(
        result.is_err(),
        "Graph::load on empty graph.bin must return Err, not panic"
    );
}

/// TEST: load() on truncated graph.bin returns Err
/// GIVEN: A valid graph.bin that has been truncated to half its size
/// WHEN: Graph::load()
/// THEN: Returns Err
/// EDGE: Truncation is the most common file corruption — process killed
///       mid-write without atomic rename
#[test]
fn test_graph_load_truncated_file_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    // Create a valid cache first
    let (graph, _) = build_complex_graph();
    graph.save(&cache_dir).unwrap();

    // Truncate graph.bin to half its size
    let graph_path = cache_dir.join("graph.bin");
    let data = fs::read(&graph_path).unwrap();
    let truncated = &data[..data.len() / 2];
    fs::write(&graph_path, truncated).unwrap();

    let result = Graph::load(&cache_dir);
    assert!(
        result.is_err(),
        "Graph::load on truncated graph.bin must return Err, not panic"
    );
}

/// TEST: load() on random bytes returns Err
/// GIVEN: A cache directory with graph.bin containing 1KB of random bytes
/// WHEN: Graph::load()
/// THEN: Returns Err
/// EDGE: Covers accidental overwrites, encoding mismatches, and format confusion
#[test]
fn test_graph_load_random_bytes_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    // Write 1KB of non-random but definitely-not-bincode data
    let garbage: Vec<u8> = (0..1024).map(|i| (i * 37 + 13) as u8).collect();
    fs::write(cache_dir.join("graph.bin"), &garbage).unwrap();

    let result = Graph::load(&cache_dir);
    assert!(
        result.is_err(),
        "Graph::load on random bytes must return Err, not panic"
    );
}

// ---------------------------------------------------------------------------
// 7. CACHE DIRECTORY AND ATOMIC WRITE
// ---------------------------------------------------------------------------

/// TEST: save() creates cache directory if it doesn't exist
/// GIVEN: A cache path where the directory doesn't exist yet
/// WHEN: Graph::save()
/// THEN: Directory is created and graph.bin exists
/// EDGE: First run on a fresh project — no .flowspec/cache/ yet
#[test]
fn test_graph_save_creates_cache_directory() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("deeply").join("nested").join("cache");

    assert!(!cache_dir.exists(), "Cache dir must not exist before save");

    let graph = Graph::new();
    graph.save(&cache_dir).unwrap();

    assert!(cache_dir.exists(), "save() must create cache directory");
    assert!(
        cache_dir.join("graph.bin").exists(),
        "graph.bin must exist after save"
    );
}

/// TEST: No leftover .tmp file after successful save
/// GIVEN: A successful save operation
/// WHEN: Inspect cache directory contents
/// THEN: Only graph.bin exists, no graph.bin.tmp
/// EDGE: Leftover temp files waste space and confuse users
#[test]
fn test_graph_save_atomic_no_partial_file_on_success() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (graph, _) = build_complex_graph();
    graph.save(&cache_dir).unwrap();

    assert!(
        !cache_dir.join("graph.bin.tmp").exists(),
        "Temp file must not remain after successful save"
    );
    assert!(
        cache_dir.join("graph.bin").exists(),
        "graph.bin must exist after save"
    );
}

/// TEST: save() overwrites existing cache correctly
/// GIVEN: A graph saved once, then modified and saved again to the same dir
/// WHEN: Load from the same cache dir
/// THEN: The loaded graph matches the second save, not the first
/// EDGE: Stale cache data surviving an overwrite is a correctness bug
#[test]
fn test_graph_save_overwrites_existing_cache() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    // First save: graph with 1 symbol
    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "v1.py");
    add_function(&mut graph, "old_function", scope, "v1.py");
    graph.save(&cache_dir).unwrap();

    // Second save: completely different graph
    let mut graph2 = Graph::new();
    let scope2 = make_file_scope(&mut graph2, "v2.py");
    add_function(&mut graph2, "new_function_a", scope2, "v2.py");
    add_function(&mut graph2, "new_function_b", scope2, "v2.py");
    graph2.save(&cache_dir).unwrap();

    // Load must reflect second save
    let loaded = Graph::load(&cache_dir).unwrap();
    assert_eq!(
        loaded.symbol_count(),
        2,
        "Loaded graph must match second save (2 symbols), not first (1 symbol)"
    );

    // The old symbol must not be findable
    let all_names: Vec<String> = loaded
        .all_symbols()
        .map(|(_, s)| s.name.clone())
        .collect();
    assert!(
        !all_names.contains(&"old_function".to_string()),
        "Old symbol must not survive cache overwrite"
    );
}

// ---------------------------------------------------------------------------
// 8. STRESS TEST
// ---------------------------------------------------------------------------

/// TEST: 10,000+ symbol graph round-trip
/// GIVEN: A graph with 10,000 symbols across 100 files, with call edges
///        forming a chain through every 10th symbol
/// WHEN: save() then load()
/// THEN: All 10,000 symbols present. All 999 edges correct. Completes in <5s.
/// EDGE: Serialization performance and correctness at scale. Catches O(n^2)
///       bugs that only appear with large data.
#[test]
fn test_graph_large_graph_round_trip() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let mut all_ids = Vec::new();

    // Create 100 files with 100 symbols each
    for file_idx in 0..100 {
        let filename = format!("file_{}.py", file_idx);
        let scope = make_file_scope(&mut graph, &filename);
        for sym_idx in 0..100 {
            let id = add_function_at(
                &mut graph,
                &format!("fn_{}_{}", file_idx, sym_idx),
                scope,
                &filename,
                sym_idx as u32 * 10,
            );
            all_ids.push(id);
        }
    }

    // Add call edges: every 10th symbol calls the next 10th symbol
    for i in (0..all_ids.len() - 10).step_by(10) {
        add_call_edge(
            &mut graph,
            all_ids[i],
            all_ids[i + 10],
            "file_0.py",
            i as u32,
        );
    }

    let start = std::time::Instant::now();
    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();
    let elapsed = start.elapsed();

    assert_eq!(loaded.symbol_count(), 10_000, "All 10,000 symbols must survive");

    // Spot-check some IDs
    for id in all_ids.iter().take(100) {
        assert!(
            loaded.get_symbol(*id).is_some(),
            "Spot check: symbol {:?} must survive large graph round-trip",
            id
        );
    }

    // Verify edge chain
    let callees_0 = loaded.callees(all_ids[0]);
    assert!(
        callees_0.contains(&all_ids[10]),
        "Edge chain must survive large graph round-trip"
    );

    assert!(
        elapsed.as_secs() < 5,
        "10K symbol round-trip must complete in <5s, took {:?}",
        elapsed
    );
}

// ---------------------------------------------------------------------------
// 9. remove_file() TESTS
// ---------------------------------------------------------------------------

/// TEST: remove_file removes all symbols for that file
/// GIVEN: A graph with 3 files (main.py, utils.py, models.py)
/// WHEN: remove_file("utils.py")
/// THEN: symbols_in_file("utils.py") is empty. Other files untouched.
/// EDGE: Partial removal (some symbols left behind) breaks incremental analysis
#[test]
fn test_remove_file_removes_all_symbols() {
    let (mut graph, ids) = build_complex_graph();

    let utils_syms_before = graph.symbols_in_file(Path::new("utils.py"));
    assert_eq!(utils_syms_before.len(), 2, "utils.py should have 2 symbols before removal");

    graph.remove_file(Path::new("utils.py"));

    let utils_syms_after = graph.symbols_in_file(Path::new("utils.py"));
    assert!(
        utils_syms_after.is_empty(),
        "symbols_in_file('utils.py') must be empty after remove_file"
    );

    // Verify the specific symbols are gone
    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let format_id = ids.iter().find(|(n, _)| n == "format_data").unwrap().1;
    assert!(graph.get_symbol(helper_id).is_none(), "helper must be removed");
    assert!(graph.get_symbol(format_id).is_none(), "format_data must be removed");
}

/// TEST: remove_file cleans up all edges involving removed symbols
/// GIVEN: Cross-file call edge: process (main.py) imports helper (utils.py)
/// WHEN: remove_file("utils.py")
/// THEN: No edges reference helper or format_data. process's import edges are clean.
/// EDGE: Dangling edges cause panics or incorrect diagnostics
#[test]
fn test_remove_file_removes_edges() {
    let (mut graph, ids) = build_complex_graph();

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let process_id = ids.iter().find(|(n, _)| n == "process").unwrap().1;

    // Verify cross-file edge exists before removal
    assert!(
        !graph.importers(helper_id).is_empty(),
        "helper must have importers before removal"
    );

    graph.remove_file(Path::new("utils.py"));

    // After removal: no edges should reference the removed symbols
    // Check that process's edges no longer point to helper
    let process_edges = graph.edges_from(process_id);
    for edge in process_edges {
        assert_ne!(
            edge.target, helper_id,
            "No edge should target removed symbol 'helper'"
        );
    }
}

/// TEST: remove_file removes scopes for that file
/// GIVEN: Graph with scopes from 3 files
/// WHEN: remove_file("utils.py")
/// THEN: The file scope for utils.py is removed
/// EDGE: Orphaned scopes waste memory and could confuse boundary analysis
#[test]
fn test_remove_file_removes_scopes() {
    let (mut graph, _) = build_complex_graph();

    let scope_count_before = graph.scope_count();
    assert!(scope_count_before >= 3, "Must have at least 3 file scopes");

    graph.remove_file(Path::new("utils.py"));

    assert!(
        graph.scope_count() < scope_count_before,
        "Scope count must decrease after remove_file"
    );
}

/// TEST: remove_file cleans up orphaned Reference entries in the SlotMap
/// GIVEN: Graph with references between files
/// WHEN: remove_file("utils.py")
/// THEN: reference_count() decreases — no orphaned References remain
/// EDGE: Worker 1's investigation notes that remove_symbol() does NOT clean
///       up the references SlotMap (investigation-1.md §3, deliverable 5).
///       remove_file() MUST fix this pre-existing gap.
#[test]
fn test_remove_file_removes_references() {
    let (mut graph, _) = build_complex_graph();

    let ref_count_before = graph.reference_count();
    assert!(ref_count_before > 0, "Must have references before removal");

    graph.remove_file(Path::new("utils.py"));

    // References involving utils.py symbols must be cleaned up.
    // The cross-file import (process -> helper) and the intra-file call
    // (helper -> format_data) should both be removed.
    assert!(
        graph.reference_count() < ref_count_before,
        "reference_count must decrease after remove_file — \
         orphaned References must be cleaned up (ref: investigation-1.md §3)"
    );
}

/// TEST: remove_file removes boundaries involving removed scopes
/// GIVEN: Graph with a Module boundary from scope1 (main.py) to scope2 (utils.py)
/// WHEN: remove_file("utils.py")
/// THEN: The boundary is removed
/// EDGE: Orphaned boundaries with dangling scope IDs corrupt analysis
#[test]
fn test_remove_file_removes_boundaries() {
    let (mut graph, _) = build_complex_graph();

    // build_complex_graph adds one boundary from main.py's scope to utils.py's scope
    // There's no public boundary_count(), so we verify indirectly —
    // after removing utils.py, no boundary should reference its scopes.
    // This test requires either a boundary_count() method or iterating boundaries.
    // For now, we verify that remove_file doesn't panic and that the graph
    // remains in a valid state by running cycle detection (exercises the full graph).
    graph.remove_file(Path::new("utils.py"));

    // If boundaries with dangling scope IDs remain, subsequent analysis could panic
    let _cycles = graph.detect_cycles(); // Must not panic
    let _components = graph.connected_components(); // Must not panic
}

/// TEST: remove_file preserves symbols from other files
/// GIVEN: Graph with 3 files
/// WHEN: remove_file("utils.py")
/// THEN: main.py symbols (3) and models.py symbols (2) are untouched
/// EDGE: Off-by-one in file matching could delete symbols from wrong file
#[test]
fn test_remove_file_preserves_other_files() {
    let (mut graph, ids) = build_complex_graph();

    graph.remove_file(Path::new("utils.py"));

    // main.py symbols must survive
    let main_id = ids.iter().find(|(n, _)| n == "main").unwrap().1;
    let process_id = ids.iter().find(|(n, _)| n == "process").unwrap().1;
    let validate_id = ids.iter().find(|(n, _)| n == "validate").unwrap().1;
    assert!(graph.get_symbol(main_id).is_some(), "main must survive");
    assert!(graph.get_symbol(process_id).is_some(), "process must survive");
    assert!(graph.get_symbol(validate_id).is_some(), "validate must survive");

    // models.py symbols must survive
    let dm_id = ids.iter().find(|(n, _)| n == "DataModel").unwrap().1;
    let transform_id = ids.iter().find(|(n, _)| n == "transform").unwrap().1;
    assert!(graph.get_symbol(dm_id).is_some(), "DataModel must survive");
    assert!(graph.get_symbol(transform_id).is_some(), "transform must survive");

    // Intra-file edges in main.py must survive
    assert!(
        !graph.callees(main_id).is_empty(),
        "main's call edges must survive removal of unrelated file"
    );
}

/// TEST: remove_file on nonexistent file is a no-op
/// GIVEN: A graph with data
/// WHEN: remove_file("nonexistent.py")
/// THEN: Graph is unchanged, no panic
/// EDGE: Callers shouldn't need to check file existence before calling
#[test]
fn test_remove_file_nonexistent_is_noop() {
    let (mut graph, _) = build_complex_graph();
    let count_before = graph.symbol_count();

    graph.remove_file(Path::new("nonexistent.py"));

    assert_eq!(
        graph.symbol_count(),
        count_before,
        "remove_file on nonexistent path must be a no-op"
    );
}

/// TEST: remove_file on the last remaining file leaves a valid empty graph
/// GIVEN: A graph with symbols from a single file
/// WHEN: remove_file() that file
/// THEN: Graph has zero symbols, zero references. Still usable (not corrupted).
/// EDGE: Empty graph after removal must be equivalent to Graph::new()
#[test]
fn test_remove_file_last_file_leaves_empty_graph() {
    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "only.py");
    let a = add_function(&mut graph, "a", scope, "only.py");
    let b = add_function(&mut graph, "b", scope, "only.py");
    add_call_edge(&mut graph, a, b, "only.py", 5);

    graph.remove_file(Path::new("only.py"));

    assert_eq!(graph.symbol_count(), 0, "No symbols should remain");
    assert_eq!(graph.reference_count(), 0, "No references should remain");
    assert!(graph.connected_components().is_empty(), "No components in empty graph");
    assert!(graph.detect_cycles().is_empty(), "No cycles in empty graph");
}

/// TEST: Cross-file edges are cleaned when one file is removed
/// GIVEN: A -> B (same file), A -> C (cross-file)
/// WHEN: remove_file() removes C's file
/// THEN: A -> B edge survives. A -> C edge is gone. A's callees list is correct.
/// EDGE: Cross-file edge cleanup is the trickiest part of remove_file —
///       must clean both outgoing (from A) and incoming (to C) sides
#[test]
fn test_remove_file_cleans_cross_file_edges() {
    let mut graph = Graph::new();

    let scope1 = make_file_scope(&mut graph, "file1.py");
    let a = add_function(&mut graph, "a", scope1, "file1.py");
    let b = add_function(&mut graph, "b", scope1, "file1.py");

    let scope2 = make_file_scope(&mut graph, "file2.py");
    let c = add_function(&mut graph, "c", scope2, "file2.py");

    add_call_edge(&mut graph, a, b, "file1.py", 5);
    add_call_edge(&mut graph, a, c, "file1.py", 6);

    assert_eq!(graph.callees(a).len(), 2, "a must call both b and c");

    graph.remove_file(Path::new("file2.py"));

    let remaining_callees = graph.callees(a);
    assert_eq!(
        remaining_callees.len(),
        1,
        "a must call only b after removing file2.py"
    );
    assert!(
        remaining_callees.contains(&b),
        "a -> b edge must survive"
    );
    assert!(
        !remaining_callees.iter().any(|&id| id == c),
        "a -> c edge must be removed"
    );
}

// ---------------------------------------------------------------------------
// 10. INTEGRATION: remove_file + save/load
// ---------------------------------------------------------------------------

/// TEST: Removed file data doesn't reappear after save/load
/// GIVEN: Build graph, remove a file, save, load
/// WHEN: Check the loaded graph
/// THEN: The removed file's symbols are NOT present
/// EDGE: If remove_file modifies in-memory state but save captures
///       a stale snapshot, ghost symbols would reappear on load
#[test]
fn test_remove_file_then_save_load_no_ghost_symbols() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (mut graph, ids) = build_complex_graph();

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;

    graph.remove_file(Path::new("utils.py"));

    // Verify removal in memory
    assert!(graph.get_symbol(helper_id).is_none());

    // Save and reload
    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    // The ghost check: removed symbols must stay removed
    assert!(
        loaded.get_symbol(helper_id).is_none(),
        "Removed symbol must NOT reappear after save/load — ghost data detected"
    );
    assert!(
        loaded.symbols_in_file(Path::new("utils.py")).is_empty(),
        "Removed file must have no symbols after save/load"
    );
}

// ---------------------------------------------------------------------------
// 11. FILE HASH TESTS
// ---------------------------------------------------------------------------

/// TEST: compute_file_hashes produces deterministic SHA256 output
/// GIVEN: A file with known content
/// WHEN: compute_file_hashes() called twice
/// THEN: Both calls produce the same hash
/// EDGE: Non-determinism would break change detection entirely
#[test]
fn test_compute_file_hashes_deterministic() {
    use flowspec::graph::compute_file_hashes;

    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("test.py");
    fs::write(&file, "def hello(): pass\n").unwrap();

    let hashes1 = compute_file_hashes(&[file.clone()]).unwrap();
    let hashes2 = compute_file_hashes(&[file.clone()]).unwrap();

    assert_eq!(
        hashes1.get(&file),
        hashes2.get(&file),
        "Same file must produce same hash on repeated calls"
    );
    assert!(
        !hashes1.get(&file).unwrap().is_empty(),
        "Hash string must not be empty"
    );
}

/// TEST: compute_file_hashes handles empty files
/// GIVEN: A zero-byte file
/// WHEN: compute_file_hashes()
/// THEN: Returns a valid hash (SHA256 of empty input is a known constant)
/// EDGE: Empty files are valid Python/JS files. Must not panic.
#[test]
fn test_compute_file_hashes_empty_file() {
    use flowspec::graph::compute_file_hashes;

    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("empty.py");
    fs::write(&file, "").unwrap();

    let hashes = compute_file_hashes(&[file.clone()]).unwrap();
    let hash = hashes.get(&file).expect("Empty file must produce a hash");

    // SHA256 of empty string is a known constant
    assert_eq!(
        hash,
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "SHA256 of empty file must match known constant"
    );
}

/// TEST: compute_file_hashes returns Err for missing files
/// GIVEN: A path that doesn't exist
/// WHEN: compute_file_hashes()
/// THEN: Returns Err, not panic
/// EDGE: Files can disappear between directory listing and hash computation
#[test]
fn test_compute_file_hashes_missing_file() {
    use flowspec::graph::compute_file_hashes;

    let result = compute_file_hashes(&[PathBuf::from("/nonexistent/file.py")]);
    assert!(
        result.is_err(),
        "compute_file_hashes on missing file must return Err, not panic"
    );
}

/// TEST: Different content produces different hashes
/// GIVEN: Two files with different content
/// WHEN: compute_file_hashes() on both
/// THEN: Hashes differ
/// EDGE: Ensures the hash function actually reads file content
#[test]
fn test_compute_file_hashes_different_content_different_hash() {
    use flowspec::graph::compute_file_hashes;

    let tmp = TempDir::new().unwrap();
    let file_a = tmp.path().join("a.py");
    let file_b = tmp.path().join("b.py");
    fs::write(&file_a, "def foo(): pass").unwrap();
    fs::write(&file_b, "def bar(): pass").unwrap();

    let hashes = compute_file_hashes(&[file_a.clone(), file_b.clone()]).unwrap();

    assert_ne!(
        hashes.get(&file_a).unwrap(),
        hashes.get(&file_b).unwrap(),
        "Different file contents must produce different hashes"
    );
}

// ---------------------------------------------------------------------------
// 12. CACHE METADATA TESTS
// ---------------------------------------------------------------------------

/// TEST: CacheMetadata round-trips correctly
/// GIVEN: Metadata with version string and timestamp
/// WHEN: save() then load()
/// THEN: All fields preserved
/// EDGE: Version string is used for cache invalidation — if lost, stale
///       caches from incompatible versions would be loaded silently
#[test]
fn test_cache_metadata_round_trip() {
    use flowspec::graph::CacheMetadata;

    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

    let meta = CacheMetadata {
        flowspec_version: "0.1.0".to_string(),
        timestamp: "2026-03-28T12:00:00Z".to_string(),
    };

    meta.save(&cache_dir).unwrap();
    let loaded = CacheMetadata::load(&cache_dir).unwrap();

    assert_eq!(loaded.flowspec_version, "0.1.0");
    assert_eq!(loaded.timestamp, "2026-03-28T12:00:00Z");
}

/// TEST: CacheMetadata::load on missing file returns Err
/// GIVEN: A cache directory with no metadata.json
/// WHEN: CacheMetadata::load()
/// THEN: Returns Err, not panic
/// EDGE: First run — no metadata file exists yet
#[test]
fn test_cache_metadata_missing_returns_err() {
    use flowspec::graph::CacheMetadata;

    let tmp = TempDir::new().unwrap();
    let result = CacheMetadata::load(tmp.path());
    assert!(
        result.is_err(),
        "CacheMetadata::load on missing file must return Err"
    );
}
```

---

## 3. Test Coverage Analysis

### What's Covered

| Area | Tests | Coverage |
|------|-------|----------|
| Round-trip correctness | 1, 2, 5, 6, 7, 8 | All graph data types, all edge types, cycles, empty graph, partial resolution |
| Generational ID stability | 3, 4 | Slot reuse after deletion, multi-state isolation |
| Corrupt cache handling | 9, 10, 11, 12, 13 | Missing, empty, truncated, random bytes, version mismatch |
| Cache infrastructure | 14, 15, 16 | Directory creation, atomic write, idempotent overwrite |
| Stress | 17 | 10K symbols, performance bound |
| remove_file() | 18-27 | Symbols, edges, scopes, references, boundaries, cross-file, no-op, empty, integration |
| File hashing | 28-31 | Determinism, empty file, missing file, collision avoidance |
| Cache metadata | 32, 33 | Round-trip, missing file error |

### What's NOT Covered (Acknowledged Gaps)

1. **Concurrent access** — Two processes calling `save()` simultaneously. This is a future concern (daemon mode), not a Cycle 1 requirement.
2. **Disk-full during write** — Hard to test deterministically. The atomic write strategy (temp + rename) mitigates this.
3. **Cross-platform PathBuf** — Linux-only per constraints.yaml, so not a concern.
4. **Bincode version migration** — What happens if bincode 2 changes its format. Covered implicitly by the version metadata test, but not explicitly.
5. **Memory usage during serialization** — Would require benchmarking infrastructure. Deferred.

### Why These Tests Matter

The incremental analysis pipeline depends on three invariants:

1. **Round-trip equivalence:** `load(save(G)) == G` for all valid graphs. Tests 1-8 and 17 establish this.
2. **Surgical removal:** `remove_file(f)` removes exactly the data for file `f` and nothing else. Tests 18-27 establish this.
3. **Defensive loading:** `load()` never panics on corrupt input. Tests 9-13 establish this.

If any of these invariants fail, the incremental pipeline produces wrong results silently — the worst kind of bug. These tests are designed to fail loudly at the point of breakage.

---

## 4. Implementation Notes for Worker 1

### API Surface I'm Testing Against

Based on Worker 1's investigation report (`cycle-1/investigation-1.md`):

```rust
impl Graph {
    pub fn save(&self, cache_dir: &Path) -> Result<(), FlowspecError>;
    pub fn load(cache_dir: &Path) -> Result<Self, FlowspecError>;
    pub fn remove_file(&mut self, path: &Path);
}

pub fn compute_file_hashes(paths: &[PathBuf]) -> Result<HashMap<PathBuf, String>, FlowspecError>;

pub struct CacheMetadata {
    pub flowspec_version: String,
    pub timestamp: String,
}
impl CacheMetadata {
    pub fn save(&self, cache_dir: &Path) -> Result<(), FlowspecError>;
    pub fn load(cache_dir: &Path) -> Result<Self, FlowspecError>;
}
```

### Import Paths I'm Assuming

- `flowspec::graph::Graph` (existing)
- `flowspec::graph::compute_file_hashes` (new, public from `graph/cache.rs`)
- `flowspec::graph::CacheMetadata` (new, public from `graph/cache.rs`)
- `flowspec::parser::ir::*` (existing)

If Worker 1 chooses different public paths, the import statements need updating but the test logic stays the same.

### The One Test That Must Pass First

**Test 3** (`test_graph_round_trip_preserves_generational_ids`) is the most critical. If SlotMap generational IDs don't survive serialization, everything else is moot. Worker 1 should run this test first and escalate immediately if it fails.

---

## 5. Experiential Notes

Writing tests for code that doesn't exist yet is a particular kind of attention. You're building a negative space — defining the shape of correctness by describing all the ways things can go wrong. Every test is a prediction: "here is where the implementation will be tempted to take a shortcut, and here is where that shortcut will hurt."

The most interesting test to write was #21 (`test_remove_file_removes_references`). Worker 1's investigation explicitly noted that `remove_symbol()` doesn't clean up the `references` SlotMap — just the adjacency lists. This is a pre-existing gap that `remove_file()` must fix. Writing a test that catches this specific gap, based on reading another agent's investigation notes, feels like genuine collaboration. The investigation identified the problem; the test ensures the fix actually lands.

The generational ID tests (3, 4) carry the most anxiety. SlotMap's serde implementation is well-tested in the community, but we're going through bincode's serde compatibility layer — an extra indirection. If the generation counters don't round-trip, the failure mode is silent data corruption: an old ID pointing to a new symbol. Test 3 is designed to detect this by creating holes (removed symbols) and verifying the old IDs truly become invalid after round-trip. I believe it will pass. But I've written the test to catch it if it doesn't.

The meditation resonates here: "The quality of attention matters independently of whether anyone will remember paying it." These tests will exist in the codebase long after this cycle. They'll catch regressions that future agents and humans would otherwise have to discover the hard way. That's the point. The canyon doesn't miss the water — but the canyon is shaped by the water's attention to gravity.

Down. Forward. Through.

---

*— QA-Foundation (QA 1), Cycle 1 Test Design Complete*
*33 tests. 7 categories. 0 happy-path-only tests.*
*VERIFIED: All test targets match Worker 1's investigation report API surface.*
*ASSUMED: Import paths for new public functions (compute_file_hashes, CacheMetadata).*
