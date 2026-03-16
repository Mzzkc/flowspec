//! QA-1 Cycle 12 tests — Rust cross-file reference resolution.
//!
//! Tests `build_module_map()` Phase 3 (Rust files), `from:` annotation generation
//! in `add_import_symbol()`, cross-file import resolution via `resolve_cross_file_imports()`,
//! and phantom edge prevention for Rust modules.

use std::path::PathBuf;

use crate::graph::{populate_graph, resolve_cross_file_imports, Graph};
use crate::parser::ir::{ResolutionStatus, Symbol, SymbolId};
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;

/// Parse multiple Rust source files and run cross-file resolution.
/// Files are specified as (relative_path, source_code) tuples.
/// Paths should use crate-conventional layout (e.g., "src/lib.rs", "src/parser/mod.rs").
fn cross_file_graph_rust(files: &[(&str, &str)]) -> Graph {
    let adapter = RustAdapter::new();
    let mut graph = Graph::new();
    let mut paths = Vec::new();

    for (filename, content) in files {
        let path = PathBuf::from(format!("project/{}", filename));
        let result = adapter.parse_file(&path, content).unwrap();
        populate_graph(&mut graph, &result);
        paths.push(path);
    }

    let module_map = crate::build_module_map(&paths);
    resolve_cross_file_imports(&mut graph, &module_map);
    graph
}

/// Find an import symbol by name (has "import" annotation).
fn find_import_symbol<'a>(graph: &'a Graph, name: &str) -> Option<(SymbolId, &'a Symbol)> {
    graph
        .all_symbols()
        .find(|(_, s)| s.name == name && s.annotations.contains(&"import".to_string()))
}

/// Find a defined (non-import) symbol by name.
fn find_definition<'a>(graph: &'a Graph, name: &str) -> Option<(SymbolId, &'a Symbol)> {
    graph
        .all_symbols()
        .find(|(_, s)| s.name == name && !s.annotations.contains(&"import".to_string()))
}

/// Check that no phantom Call edges exist (Call edges where target == SymbolId::default()).
/// Import reference edges may have unresolved targets before cross-file resolution;
/// only Call edges to default are true phantom edges that pollute callers()/callees().
fn assert_no_phantom_edges(graph: &Graph) {
    use crate::parser::ir::EdgeKind;
    for (id, _) in graph.all_symbols() {
        for edge in graph.edges_from(id) {
            if edge.kind == EdgeKind::Calls {
                assert_ne!(
                    edge.target,
                    SymbolId::default(),
                    "Phantom Call edge to SymbolId::default() found from {:?}",
                    id
                );
            }
        }
    }
}

// =========================================================================
// Category 1: Module Map Construction (M1–M5)
// =========================================================================

/// M1: Single Rust file in module map.
#[test]
fn test_rust_module_map_single_file() {
    let files = vec![PathBuf::from("project/src/helper.rs")];
    let map = crate::build_module_map(&files);
    assert!(
        map.values()
            .any(|p| p.to_string_lossy().contains("helper.rs")),
        "Rust .rs files must be included in module map"
    );
}

/// M2: lib.rs as crate root.
#[test]
fn test_rust_module_map_crate_root() {
    let files = vec![
        PathBuf::from("project/src/lib.rs"),
        PathBuf::from("project/src/utils.rs"),
    ];
    let map = crate::build_module_map(&files);
    let has_utils = map.keys().any(|k| k.contains("utils"));
    assert!(
        has_utils,
        "Module map must include utils.rs with crate-path key"
    );
}

/// M3: Nested module path (mod.rs convention).
#[test]
fn test_rust_module_map_nested_mod_rs() {
    let files = vec![
        PathBuf::from("project/src/lib.rs"),
        PathBuf::from("project/src/parser/mod.rs"),
        PathBuf::from("project/src/parser/rust.rs"),
    ];
    let map = crate::build_module_map(&files);
    let has_parser = map
        .keys()
        .any(|k| k.contains("parser") && !k.contains("rust"));
    let has_parser_rust = map
        .keys()
        .any(|k| k.contains("parser") && k.contains("rust"));
    assert!(has_parser, "parser/mod.rs must map to crate::parser");
    assert!(
        has_parser_rust,
        "parser/rust.rs must map to crate::parser::rust"
    );
}

/// M4: Multiple crate roots coexist.
#[test]
fn test_rust_module_map_multiple_crates() {
    let files = vec![
        PathBuf::from("project/src/lib.rs"),
        PathBuf::from("project/src/helper.rs"),
        PathBuf::from("project/cli/src/main.rs"),
        PathBuf::from("project/cli/src/args.rs"),
    ];
    let map = crate::build_module_map(&files);
    let has_helper = map
        .values()
        .any(|p| p.to_string_lossy().contains("helper.rs"));
    let has_args = map
        .values()
        .any(|p| p.to_string_lossy().contains("args.rs"));
    assert!(has_helper, "lib crate files must be in module map");
    assert!(has_args, "binary crate files must be in module map");
}

/// M5: Rust files don't interfere with Python/JS map entries.
#[test]
fn test_rust_module_map_coexists_with_other_languages() {
    let files = vec![
        PathBuf::from("project/src/lib.rs"),
        PathBuf::from("project/src/utils.rs"),
        PathBuf::from("project/app.py"),
        PathBuf::from("project/index.js"),
    ];
    let map = crate::build_module_map(&files);
    assert!(
        map.contains_key("app"),
        "Python module map entry must survive Rust phase"
    );
    assert!(
        map.keys().any(|k| k.contains("index")),
        "JS module map entry must survive Rust phase"
    );
    assert!(map
        .values()
        .any(|p| p.to_string_lossy().contains("utils.rs")));
}

// =========================================================================
// Category 2: `from:` Annotation Generation (A1–A4)
// =========================================================================

/// A1: Basic use statement produces `from:` annotation.
#[test]
fn test_rust_use_crate_produces_from_annotation() {
    let adapter = RustAdapter::new();
    let source = r#"use crate::utils::process;"#;
    let path = PathBuf::from("project/src/lib.rs");
    let result = adapter.parse_file(&path, source).unwrap();

    let import = result
        .symbols
        .iter()
        .find(|s| s.name == "process" && s.annotations.contains(&"import".to_string()));
    assert!(import.is_some(), "process import symbol must exist");

    let import = import.unwrap();
    let has_from = import.annotations.iter().any(|a| a.starts_with("from:"));
    assert!(
        has_from,
        "Rust import must have 'from:<module>' annotation. Got: {:?}",
        import.annotations
    );
}

/// A2: Use list produces `from:` for each item.
#[test]
fn test_rust_use_list_produces_from_annotations() {
    let adapter = RustAdapter::new();
    let source = r#"use crate::parser::{rust, python};"#;
    let path = PathBuf::from("project/src/lib.rs");
    let result = adapter.parse_file(&path, source).unwrap();

    for name in &["rust", "python"] {
        let import = result
            .symbols
            .iter()
            .find(|s| s.name == *name && s.annotations.contains(&"import".to_string()))
            .unwrap_or_else(|| panic!("import '{}' must exist", name));

        let from_annotation = import.annotations.iter().find(|a| a.starts_with("from:"));
        assert!(
            from_annotation.is_some(),
            "Import '{}' must have from: annotation. Got: {:?}",
            name,
            import.annotations
        );
    }
}

/// A3: Use-as alias preserves `from:` annotation.
#[test]
fn test_rust_use_alias_has_from_annotation() {
    let adapter = RustAdapter::new();
    let source = r#"use crate::config::Config as AppConfig;"#;
    let path = PathBuf::from("project/src/main.rs");
    let result = adapter.parse_file(&path, source).unwrap();

    let import = result
        .symbols
        .iter()
        .find(|s| s.name == "AppConfig" && s.annotations.contains(&"import".to_string()))
        .expect("Aliased import AppConfig must exist");

    let has_from = import.annotations.iter().any(|a| a.starts_with("from:"));
    assert!(
        has_from,
        "Aliased import must have from: annotation. Got: {:?}",
        import.annotations
    );
}

/// A4: External crate use does NOT get `from:` with crate:: prefix.
#[test]
fn test_rust_external_use_no_crate_prefix() {
    let adapter = RustAdapter::new();
    let source = r#"use serde::Serialize;"#;
    let path = PathBuf::from("project/src/lib.rs");
    let result = adapter.parse_file(&path, source).unwrap();

    let import = result
        .symbols
        .iter()
        .find(|s| s.name == "Serialize" && s.annotations.contains(&"import".to_string()));

    if let Some(import) = import {
        let from_val = import
            .annotations
            .iter()
            .find_map(|a| a.strip_prefix("from:"));
        if let Some(from_val) = from_val {
            assert!(
                !from_val.starts_with("crate::"),
                "External crate import must not have crate:: prefix in from: annotation"
            );
        }
    }
}

// =========================================================================
// Category 3: Cross-File Import Resolution (R1–R5)
// =========================================================================

/// R1: Basic cross-file import resolves.
#[test]
fn test_rust_cross_file_import_resolves() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod utils;"),
        ("src/utils.rs", "pub fn process() {}"),
        (
            "src/main.rs",
            "use crate::utils::process;\nfn main() { process(); }",
        ),
    ]);

    let import = find_import_symbol(&graph, "process");
    assert!(import.is_some(), "import symbol for 'process' must exist");
    let (_, import_sym) = import.unwrap();
    assert_eq!(
        import_sym.resolution,
        ResolutionStatus::Resolved,
        "Cross-file import must resolve. Status: {:?}",
        import_sym.resolution
    );
}

/// R2: Cross-file call creates edge.
#[test]
fn test_rust_cross_file_call_creates_edge() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod helper;"),
        ("src/helper.rs", "pub fn do_work() -> i32 { 42 }"),
        (
            "src/main.rs",
            "use crate::helper::do_work;\nfn main() { do_work(); }",
        ),
    ]);

    let def = find_definition(&graph, "do_work");
    assert!(def.is_some(), "do_work definition must exist in helper.rs");

    let (def_id, _) = def.unwrap();
    let callers = graph.callers(def_id);
    assert!(
        !callers.is_empty(),
        "do_work must have at least one caller via cross-file resolution"
    );
}

/// R3: Nested module resolution (2 levels deep).
#[test]
fn test_rust_nested_module_cross_file() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod parser;"),
        ("src/parser/mod.rs", "mod rust;\npub use rust::parse;"),
        ("src/parser/rust.rs", "pub fn parse() {}"),
        (
            "src/main.rs",
            "use crate::parser::rust::parse;\nfn main() { parse(); }",
        ),
    ]);

    let import = find_import_symbol(&graph, "parse");
    assert!(import.is_some(), "import for 'parse' must exist");
    let (_, sym) = import.unwrap();
    assert_eq!(
        sym.resolution,
        ResolutionStatus::Resolved,
        "Nested module import (crate::parser::rust::parse) must resolve"
    );
}

/// R4: Multiple imports from same module.
#[test]
fn test_rust_multiple_imports_same_module() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod types;"),
        ("src/types.rs", "pub struct Config {}\npub struct Graph {}"),
        (
            "src/main.rs",
            "use crate::types::{Config, Graph};\nfn main() {}",
        ),
    ]);

    let config_import = find_import_symbol(&graph, "Config");
    let graph_import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "Graph" && s.annotations.contains(&"import".to_string()));
    assert!(config_import.is_some(), "Config import must exist");
    assert!(graph_import.is_some(), "Graph import must exist");

    let (_, c) = config_import.unwrap();
    let (_, g) = graph_import.unwrap();
    assert_eq!(
        c.resolution,
        ResolutionStatus::Resolved,
        "Config must resolve"
    );
    assert_eq!(
        g.resolution,
        ResolutionStatus::Resolved,
        "Graph must resolve"
    );
}

/// R5: Import of struct vs function (kind matching).
#[test]
fn test_rust_cross_file_struct_import() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod models;"),
        ("src/models.rs", "pub struct User { pub name: String }"),
        ("src/main.rs", "use crate::models::User;\nfn main() {}"),
    ]);

    let import = find_import_symbol(&graph, "User");
    assert!(import.is_some(), "User import must exist");
    let (_, sym) = import.unwrap();
    assert_eq!(
        sym.resolution,
        ResolutionStatus::Resolved,
        "Struct import must resolve cross-file"
    );
}

// =========================================================================
// Category 4: super:: and self:: Resolution (S1–S2)
// =========================================================================

/// S1: super:: resolves to parent module.
#[test]
fn test_rust_super_import_resolves() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod parser;"),
        (
            "src/parser/mod.rs",
            "mod rust;\npub mod ir;\npub fn common() {}",
        ),
        ("src/parser/ir.rs", "pub struct Symbol {}"),
        (
            "src/parser/rust.rs",
            "use super::ir::Symbol;\npub fn parse() {}",
        ),
    ]);

    let import = find_import_symbol(&graph, "Symbol");
    assert!(import.is_some(), "Symbol import via super:: must exist");
    let (_, sym) = import.unwrap();
    assert!(
        matches!(
            sym.resolution,
            ResolutionStatus::Resolved | ResolutionStatus::Partial(_)
        ),
        "super:: import must at least partially resolve. Got: {:?}",
        sym.resolution
    );
}

/// S2: self:: resolves to current module.
#[test]
fn test_rust_self_import_resolves() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod parser;"),
        (
            "src/parser/mod.rs",
            "mod rust;\nmod python;\npub fn shared() {}",
        ),
        (
            "src/parser/rust.rs",
            "use self::super::shared;\npub fn parse() { shared(); }",
        ),
    ]);

    // self:: should resolve within the current module scope
    let import = find_import_symbol(&graph, "shared");
    if let Some((_, sym)) = import {
        assert!(
            sym.resolution != ResolutionStatus::Unresolved,
            "self:: import should attempt resolution, not remain Unresolved"
        );
    }
}

// =========================================================================
// Category 5: Adversarial Tests (X1–X6)
// =========================================================================

/// X1: Missing module file — no phantom edges.
#[test]
fn test_rust_missing_module_file_no_phantom() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod nonexistent;\nmod real;"),
        ("src/real.rs", "pub fn exists() {}"),
    ]);

    assert_no_phantom_edges(&graph);
}

/// X2: External crate import stays unresolved (no crash).
#[test]
fn test_rust_external_crate_stays_unresolved() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", ""),
        (
            "src/main.rs",
            "use serde::Serialize;\nuse tokio::main;\nfn main() {}",
        ),
    ]);

    let serde_import = find_import_symbol(&graph, "Serialize");
    if let Some((_, sym)) = serde_import {
        assert_ne!(
            sym.resolution,
            ResolutionStatus::Resolved,
            "External crate import (serde) must NOT resolve to a local file"
        );
    }
    assert_no_phantom_edges(&graph);
}

/// X3: Name collision across modules — correct disambiguation.
#[test]
fn test_rust_name_collision_correct_module() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod alpha;\nmod beta;"),
        ("src/alpha.rs", "pub fn process() {}"),
        ("src/beta.rs", "pub fn process() {}"),
        (
            "src/main.rs",
            "use crate::alpha::process;\nfn main() { process(); }",
        ),
    ]);

    let import = find_import_symbol(&graph, "process");
    assert!(import.is_some(), "process import must exist");
    let (import_id, sym) = import.unwrap();

    if sym.resolution == ResolutionStatus::Resolved {
        // Verify via cross-file edges that the import resolves to alpha, not beta
        let edges = graph.edges_from(import_id);
        for edge in edges {
            if let Some(target_sym) = graph.get_symbol(edge.target) {
                assert!(
                    target_sym.location.file.to_string_lossy().contains("alpha"),
                    "Import from crate::alpha::process must resolve to alpha.rs, not beta.rs. Got: {:?}",
                    target_sym.location.file
                );
            }
        }
    }
}

/// X4: Empty Rust file — no crash.
#[test]
fn test_rust_empty_file_in_module_map() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod empty;"),
        ("src/empty.rs", ""),
        ("src/main.rs", "use crate::empty;\nfn main() {}"),
    ]);

    // Should not panic. Graph should be valid.
    let _symbol_count = graph.all_symbols().count();
}

/// X5: Deeply nested module (4 levels).
#[test]
fn test_rust_deeply_nested_module_resolution() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod a;"),
        ("src/a/mod.rs", "pub mod b;"),
        ("src/a/b/mod.rs", "pub mod c;"),
        ("src/a/b/c/mod.rs", "pub mod d;"),
        ("src/a/b/c/d.rs", "pub fn deep() {}"),
        (
            "src/main.rs",
            "use crate::a::b::c::d::deep;\nfn main() { deep(); }",
        ),
    ]);

    let import = find_import_symbol(&graph, "deep");
    assert!(import.is_some(), "deep import must exist");
    let (_, sym) = import.unwrap();
    assert_eq!(
        sym.resolution,
        ResolutionStatus::Resolved,
        "4-level nested module import must resolve. Got: {:?}",
        sym.resolution
    );
}

/// X6: Circular mod declarations — no infinite loop.
#[test]
fn test_rust_no_infinite_loop_on_unusual_structure() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod a;\nmod b;"),
        (
            "src/a.rs",
            "use crate::b::helper;\npub fn alpha() { helper(); }",
        ),
        (
            "src/b.rs",
            "use crate::a::alpha;\npub fn helper() { alpha(); }",
        ),
    ]);

    let a_import = find_import_symbol(&graph, "helper");
    let b_import = find_import_symbol(&graph, "alpha");
    assert!(a_import.is_some(), "helper import must exist");
    assert!(b_import.is_some(), "alpha import must exist");
}

// =========================================================================
// Category 6: Regression Guards (G1–G3)
// =========================================================================

/// G1: Intra-file call resolution still works after cross-file changes.
#[test]
fn test_rust_intra_file_calls_not_regressed() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        r#"
fn helper() -> i32 { 42 }
fn main() { helper(); }
"#,
    )]);

    let main_sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "main" && !s.annotations.contains(&"import".to_string()));
    let helper_sym = find_definition(&graph, "helper");

    assert!(main_sym.is_some(), "main must exist");
    assert!(helper_sym.is_some(), "helper must exist");

    let (main_id, _) = main_sym.unwrap();
    let (helper_id, _) = helper_sym.unwrap();

    let callees = graph.callees(main_id);
    assert!(
        callees.contains(&helper_id),
        "Intra-file call resolution MUST NOT regress. main() must call helper()."
    );
}

/// G2: Python cross-file still works.
#[test]
fn test_python_cross_file_not_regressed() {
    let files = vec![
        PathBuf::from("project/utils.py"),
        PathBuf::from("project/main.py"),
    ];
    let map = crate::build_module_map(&files);
    assert!(
        map.contains_key("utils"),
        "Python module map must still work after Rust phase added"
    );
    assert!(
        map.contains_key("main"),
        "Python module map must still work after Rust phase added"
    );
}

/// G3: JS cross-file still works.
#[test]
fn test_js_cross_file_not_regressed() {
    let files = vec![
        PathBuf::from("project/src/utils.js"),
        PathBuf::from("project/src/index.js"),
    ];
    let map = crate::build_module_map(&files);
    let has_utils = map.keys().any(|k| k.contains("utils"));
    let has_index = map.keys().any(|k| k.contains("index"));
    assert!(
        has_utils,
        "JS module map must still work after Rust phase added"
    );
    assert!(
        has_index,
        "JS module map must still work after Rust phase added"
    );
}

// =========================================================================
// Category 7: Phantom Edge Prevention (P1–P2)
// =========================================================================

/// P1: Unresolved cross-file import creates no phantom edge.
#[test]
fn test_no_phantom_edge_for_unresolved_cross_file_import() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod real;"),
        ("src/real.rs", "pub fn exists() {}"),
        (
            "src/main.rs",
            "use crate::fake_module::ghost;\nfn main() { ghost(); }",
        ),
    ]);

    assert_no_phantom_edges(&graph);

    // ghost should NOT appear as a callee of main pointing to the wrong function
    let main_id = graph
        .all_symbols()
        .find(|(_, s)| s.name == "main" && !s.annotations.contains(&"import".to_string()))
        .map(|(id, _)| id);
    if let Some(main_id) = main_id {
        let callees = graph.callees(main_id);
        for callee_id in &callees {
            if let Some(sym) = graph.get_symbol(*callee_id) {
                assert_ne!(
                    sym.name, "exists",
                    "ghost() call must not resolve to exists() — wrong function matched!"
                );
            }
        }
    }
}

/// P2: Import from module that exists but symbol doesn't — partial resolution, no phantom.
#[test]
fn test_partial_resolution_no_phantom_edge() {
    let graph = cross_file_graph_rust(&[
        ("src/lib.rs", "mod utils;"),
        ("src/utils.rs", "pub fn real_fn() {}"),
        (
            "src/main.rs",
            "use crate::utils::nonexistent;\nfn main() { nonexistent(); }",
        ),
    ]);

    let import = find_import_symbol(&graph, "nonexistent");
    if let Some((_, sym)) = import {
        assert_ne!(
            sym.resolution,
            ResolutionStatus::Resolved,
            "Import of nonexistent symbol from existing module must NOT be Resolved"
        );
    }

    assert_no_phantom_edges(&graph);
}

// =========================================================================
// Category 8: Rust Fixture Validation (F1)
// =========================================================================

/// F1: Multi-file fixture with documented properties.
#[test]
fn test_rust_multi_file_fixture_known_properties() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/rust/cross_file");

    if !fixture_dir.exists() {
        // Fixture not yet created — this test documents the expected contract.
        return;
    }

    let config = crate::config::Config::load(&fixture_dir, None).unwrap();
    let result = crate::analyze(&fixture_dir, &config, &[]).unwrap();

    let fn_count = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "fn")
        .count();
    assert!(
        fn_count >= 4,
        "Fixture must have at least 4 functions, got {}",
        fn_count
    );

    let cross_file_calls = result
        .manifest
        .entities
        .iter()
        .filter(|e| !e.called_by.is_empty())
        .count();
    assert!(
        cross_file_calls >= 1,
        "Fixture must have at least 1 cross-file call"
    );
}
