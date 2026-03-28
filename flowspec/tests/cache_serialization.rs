//! Integration tests for graph serialization, cache integrity, and incremental operations.
//!
//! These tests are adversarial by design — they target failure modes, not happy paths.
//! Written BEFORE implementation (TDD) by QA-Foundation, Cycle 1.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use flowspec::graph::{compute_file_hashes, CacheMetadata, Graph};
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

#[test]
fn test_graph_round_trip_complex_graph() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (original, ids) = build_complex_graph();

    original.save(&cache_dir).expect("save must succeed");
    let loaded = Graph::load(&cache_dir).expect("load must succeed");

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

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let process_imports = loaded.importers(helper_id);
    assert!(
        process_imports.contains(&process_id),
        "Cross-file import edge (process -> helper) must survive round-trip"
    );

    let dm_id = ids.iter().find(|(n, _)| n == "DataModel").unwrap().1;
    let dm = loaded.get_symbol(dm_id).unwrap();
    assert_eq!(
        dm.annotations,
        vec!["dataclass".to_string()],
        "Annotations must survive round-trip"
    );

    let transform_id = ids.iter().find(|(n, _)| n == "transform").unwrap().1;
    let transform = loaded.get_symbol(transform_id).unwrap();
    assert_eq!(
        transform.signature,
        Some("(self, data: list) -> dict".to_string()),
        "Signature must survive round-trip"
    );

    let main_file_syms = loaded.symbols_in_file(Path::new("main.py"));
    assert_eq!(
        main_file_syms.len(),
        3,
        "file_symbols mapping for main.py must survive round-trip"
    );
}

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
        let sym = loaded.get_symbol(*id).unwrap_or_else(|| {
            panic!(
                "Symbol with kind {:?} must survive round-trip",
                expected_kind
            )
        });
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

#[test]
fn test_graph_round_trip_preserves_generational_ids() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "gen.py");

    let mut first_gen_ids = Vec::new();
    for i in 0..10 {
        let id = add_function_at(&mut graph, &format!("fn_{}", i), scope, "gen.py", i * 10);
        first_gen_ids.push(id);
    }

    let removed_ids: Vec<SymbolId> = first_gen_ids
        .iter()
        .enumerate()
        .filter(|(i, _)| i % 2 == 0)
        .map(|(_, id)| *id)
        .collect();
    for id in &removed_ids {
        graph.remove_symbol(*id);
    }

    let mut second_gen_ids = Vec::new();
    for i in 0..5 {
        let id = add_function_at(
            &mut graph,
            &format!("new_fn_{}", i),
            scope,
            "gen.py",
            100 + i * 10,
        );
        second_gen_ids.push(id);
    }

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    for (i, id) in first_gen_ids.iter().enumerate() {
        if i % 2 != 0 {
            let sym = loaded
                .get_symbol(*id)
                .unwrap_or_else(|| panic!("First-gen symbol fn_{} must survive round-trip", i));
            assert_eq!(sym.name, format!("fn_{}", i));
        }
    }

    for id in &removed_ids {
        assert!(
            loaded.get_symbol(*id).is_none(),
            "Removed symbol ID {:?} must NOT resolve after round-trip — \
             if it does, generational IDs are corrupted",
            id
        );
    }

    for (i, id) in second_gen_ids.iter().enumerate() {
        let sym = loaded
            .get_symbol(*id)
            .unwrap_or_else(|| panic!("Second-gen symbol new_fn_{} must survive round-trip", i));
        assert_eq!(sym.name, format!("new_fn_{}", i));
    }

    assert_eq!(loaded.symbol_count(), 10);
}

#[test]
fn test_graph_round_trip_after_deletions_no_id_collision() {
    let tmp = TempDir::new().unwrap();
    let cache_a = tmp.path().join("cache_a");
    let cache_b = tmp.path().join("cache_b");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "collision.py");

    let id_alpha = add_function(&mut graph, "alpha", scope, "collision.py");
    let id_beta = add_function(&mut graph, "beta", scope, "collision.py");

    graph.save(&cache_a).unwrap();

    graph.remove_symbol(id_alpha);
    let id_gamma = add_function(&mut graph, "gamma", scope, "collision.py");

    graph.save(&cache_b).unwrap();

    let loaded_a = Graph::load(&cache_a).unwrap();
    let loaded_b = Graph::load(&cache_b).unwrap();

    assert!(
        loaded_a.get_symbol(id_alpha).is_some(),
        "State A must have alpha"
    );
    assert!(
        loaded_a.get_symbol(id_beta).is_some(),
        "State A must have beta"
    );
    assert!(
        loaded_a.get_symbol(id_gamma).is_none(),
        "State A must NOT have gamma (didn't exist yet)"
    );

    assert!(
        loaded_b.get_symbol(id_alpha).is_none(),
        "State B must NOT have alpha (was removed)"
    );
    assert!(
        loaded_b.get_symbol(id_beta).is_some(),
        "State B must have beta"
    );
    assert!(
        loaded_b.get_symbol(id_gamma).is_some(),
        "State B must have gamma"
    );
}

// ---------------------------------------------------------------------------
// 3. EDGE/REFERENCE BACK-POINTER PRESERVATION
// ---------------------------------------------------------------------------

#[test]
fn test_graph_round_trip_preserves_edge_reference_ids() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "refs.py");
    let a = add_function(&mut graph, "caller", scope, "refs.py");
    let b = add_function(&mut graph, "callee", scope, "refs.py");
    add_call_edge(&mut graph, a, b, "refs.py", 5);

    let original_edges = graph.edges_from(a);
    assert_eq!(
        original_edges.len(),
        1,
        "Must have exactly one outgoing edge"
    );
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

    let before_members: HashSet<SymbolId> = cycles_before
        .iter()
        .flat_map(|c| c.iter().copied())
        .collect();
    let after_members: HashSet<SymbolId> = cycles_after
        .iter()
        .flat_map(|c| c.iter().copied())
        .collect();
    assert_eq!(
        before_members, after_members,
        "Cycle members must be identical after round-trip"
    );
}

// ---------------------------------------------------------------------------
// 5. EDGE CASES
// ---------------------------------------------------------------------------

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
        resolution: ResolutionStatus::Partial(
            "dynamic dispatch — could be any of 3 classes".to_string(),
        ),
        scope,
        annotations: vec!["overload".to_string(), "deprecated".to_string()],
    });

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

    let sym = loaded
        .get_symbol(id)
        .expect("Symbol must survive round-trip");
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

#[test]
fn test_graph_load_truncated_file_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (graph, _) = build_complex_graph();
    graph.save(&cache_dir).unwrap();

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

#[test]
fn test_graph_load_random_bytes_returns_err() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");
    fs::create_dir_all(&cache_dir).unwrap();

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

#[test]
fn test_graph_save_overwrites_existing_cache() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let scope = make_file_scope(&mut graph, "v1.py");
    add_function(&mut graph, "old_function", scope, "v1.py");
    graph.save(&cache_dir).unwrap();

    let mut graph2 = Graph::new();
    let scope2 = make_file_scope(&mut graph2, "v2.py");
    add_function(&mut graph2, "new_function_a", scope2, "v2.py");
    add_function(&mut graph2, "new_function_b", scope2, "v2.py");
    graph2.save(&cache_dir).unwrap();

    let loaded = Graph::load(&cache_dir).unwrap();
    assert_eq!(
        loaded.symbol_count(),
        2,
        "Loaded graph must match second save (2 symbols), not first (1 symbol)"
    );

    let all_names: Vec<String> = loaded.all_symbols().map(|(_, s)| s.name.clone()).collect();
    assert!(
        !all_names.contains(&"old_function".to_string()),
        "Old symbol must not survive cache overwrite"
    );
}

// ---------------------------------------------------------------------------
// 8. STRESS TEST
// ---------------------------------------------------------------------------

#[test]
fn test_graph_large_graph_round_trip() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let mut graph = Graph::new();
    let mut all_ids = Vec::new();

    for file_idx in 0..100 {
        let filename = format!("file_{}.py", file_idx);
        let scope = make_file_scope(&mut graph, &filename);
        for sym_idx in 0..100 {
            let id = add_function_at(
                &mut graph,
                &format!("fn_{}_{}", file_idx, sym_idx),
                scope,
                &filename,
                sym_idx * 10,
            );
            all_ids.push(id);
        }
    }

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

    assert_eq!(
        loaded.symbol_count(),
        10_000,
        "All 10,000 symbols must survive"
    );

    for id in all_ids.iter().take(100) {
        assert!(
            loaded.get_symbol(*id).is_some(),
            "Spot check: symbol {:?} must survive large graph round-trip",
            id
        );
    }

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

#[test]
fn test_remove_file_removes_all_symbols() {
    let (mut graph, ids) = build_complex_graph();

    let utils_syms_before = graph.symbols_in_file(Path::new("utils.py"));
    assert_eq!(
        utils_syms_before.len(),
        2,
        "utils.py should have 2 symbols before removal"
    );

    graph.remove_file(Path::new("utils.py"));

    let utils_syms_after = graph.symbols_in_file(Path::new("utils.py"));
    assert!(
        utils_syms_after.is_empty(),
        "symbols_in_file('utils.py') must be empty after remove_file"
    );

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let format_id = ids.iter().find(|(n, _)| n == "format_data").unwrap().1;
    assert!(
        graph.get_symbol(helper_id).is_none(),
        "helper must be removed"
    );
    assert!(
        graph.get_symbol(format_id).is_none(),
        "format_data must be removed"
    );
}

#[test]
fn test_remove_file_removes_edges() {
    let (mut graph, ids) = build_complex_graph();

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;
    let process_id = ids.iter().find(|(n, _)| n == "process").unwrap().1;

    assert!(
        !graph.importers(helper_id).is_empty(),
        "helper must have importers before removal"
    );

    graph.remove_file(Path::new("utils.py"));

    let process_edges = graph.edges_from(process_id);
    for edge in process_edges {
        assert_ne!(
            edge.target, helper_id,
            "No edge should target removed symbol 'helper'"
        );
    }
}

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

#[test]
fn test_remove_file_removes_references() {
    let (mut graph, _) = build_complex_graph();

    let ref_count_before = graph.reference_count();
    assert!(ref_count_before > 0, "Must have references before removal");

    graph.remove_file(Path::new("utils.py"));

    assert!(
        graph.reference_count() < ref_count_before,
        "reference_count must decrease after remove_file — \
         orphaned References must be cleaned up"
    );
}

#[test]
fn test_remove_file_removes_boundaries() {
    let (mut graph, _) = build_complex_graph();

    graph.remove_file(Path::new("utils.py"));

    let _cycles = graph.detect_cycles();
    let _components = graph.connected_components();
}

#[test]
fn test_remove_file_preserves_other_files() {
    let (mut graph, ids) = build_complex_graph();

    graph.remove_file(Path::new("utils.py"));

    let main_id = ids.iter().find(|(n, _)| n == "main").unwrap().1;
    let process_id = ids.iter().find(|(n, _)| n == "process").unwrap().1;
    let validate_id = ids.iter().find(|(n, _)| n == "validate").unwrap().1;
    assert!(graph.get_symbol(main_id).is_some(), "main must survive");
    assert!(
        graph.get_symbol(process_id).is_some(),
        "process must survive"
    );
    assert!(
        graph.get_symbol(validate_id).is_some(),
        "validate must survive"
    );

    let dm_id = ids.iter().find(|(n, _)| n == "DataModel").unwrap().1;
    let transform_id = ids.iter().find(|(n, _)| n == "transform").unwrap().1;
    assert!(graph.get_symbol(dm_id).is_some(), "DataModel must survive");
    assert!(
        graph.get_symbol(transform_id).is_some(),
        "transform must survive"
    );

    assert!(
        !graph.callees(main_id).is_empty(),
        "main's call edges must survive removal of unrelated file"
    );
}

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
    assert!(
        graph.connected_components().is_empty(),
        "No components in empty graph"
    );
    assert!(graph.detect_cycles().is_empty(), "No cycles in empty graph");
}

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
    assert!(remaining_callees.contains(&b), "a -> b edge must survive");
    assert!(
        !remaining_callees.iter().any(|&id| id == c),
        "a -> c edge must be removed"
    );
}

// ---------------------------------------------------------------------------
// 10. INTEGRATION: remove_file + save/load
// ---------------------------------------------------------------------------

#[test]
fn test_remove_file_then_save_load_no_ghost_symbols() {
    let tmp = TempDir::new().unwrap();
    let cache_dir = tmp.path().join("cache");

    let (mut graph, ids) = build_complex_graph();

    let helper_id = ids.iter().find(|(n, _)| n == "helper").unwrap().1;

    graph.remove_file(Path::new("utils.py"));
    assert!(graph.get_symbol(helper_id).is_none());

    graph.save(&cache_dir).unwrap();
    let loaded = Graph::load(&cache_dir).unwrap();

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

#[test]
fn test_compute_file_hashes_deterministic() {
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

#[test]
fn test_compute_file_hashes_empty_file() {
    let tmp = TempDir::new().unwrap();
    let file = tmp.path().join("empty.py");
    fs::write(&file, "").unwrap();

    let hashes = compute_file_hashes(&[file.clone()]).unwrap();
    let hash = hashes.get(&file).expect("Empty file must produce a hash");

    assert_eq!(
        hash, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        "SHA256 of empty file must match known constant"
    );
}

#[test]
fn test_compute_file_hashes_missing_file() {
    let result = compute_file_hashes(&[PathBuf::from("/nonexistent/file.py")]);
    assert!(
        result.is_err(),
        "compute_file_hashes on missing file must return Err, not panic"
    );
}

#[test]
fn test_compute_file_hashes_different_content_different_hash() {
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

#[test]
fn test_cache_metadata_round_trip() {
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

#[test]
fn test_cache_metadata_missing_returns_err() {
    let tmp = TempDir::new().unwrap();
    let result = CacheMetadata::load(tmp.path());
    assert!(
        result.is_err(),
        "CacheMetadata::load on missing file must return Err"
    );
}
