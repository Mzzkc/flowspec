// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Test utilities — mock graph builders matching fixture file planted facts.
//!
//! Each builder constructs a Graph that exactly matches the documented
//! planted facts for a fixture Python file. These are used by pattern
//! detector unit tests until the real Python parser exists.

use std::path::PathBuf;

use crate::graph::Graph;
use crate::parser::ir::*;

/// Create a Symbol with common defaults.
pub fn make_symbol(name: &str, kind: SymbolKind, vis: Visibility, file: &str, line: u32) -> Symbol {
    Symbol {
        id: SymbolId::default(),
        kind,
        name: name.to_string(),
        qualified_name: format!(
            "{}::{}",
            file.trim_end_matches(".py").trim_end_matches(".rs"),
            name
        ),
        visibility: vis,
        signature: None,
        location: Location {
            file: PathBuf::from(file),
            line,
            column: 1,
            end_line: line + 3,
            end_column: 1,
        },
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    }
}

/// Create an import Symbol (annotated with "import").
pub fn make_import(name: &str, file: &str, line: u32) -> Symbol {
    let mut sym = make_symbol(name, SymbolKind::Variable, Visibility::Public, file, line);
    sym.annotations.push("import".to_string());
    sym
}

/// Create an entry point Symbol (annotated with "entry_point").
pub fn make_entry_point(
    name: &str,
    kind: SymbolKind,
    vis: Visibility,
    file: &str,
    line: u32,
) -> Symbol {
    let mut sym = make_symbol(name, kind, vis, file, line);
    sym.annotations.push("entry_point".to_string());
    sym
}

/// Add a reference edge between two symbols in the graph.
pub fn add_ref(graph: &mut Graph, from: SymbolId, to: SymbolId, kind: ReferenceKind, file: &str) {
    graph.add_reference(Reference {
        id: ReferenceId::default(),
        from,
        to,
        kind,
        location: Location {
            file: PathBuf::from(file),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        },
        resolution: ResolutionStatus::Resolved,
    });
}

// ---------------------------------------------------------------------------
// Fixture graph builders
// ---------------------------------------------------------------------------

/// dead_code.py: unused_helper (0 callers, private), _private_util (0 callers),
/// active_function (called by main_handler), main_handler (entry point).
pub fn build_dead_code_graph() -> Graph {
    let mut g = Graph::new();
    let f = "dead_code.py";

    let s1 = g.add_symbol(make_symbol(
        "unused_helper",
        SymbolKind::Function,
        Visibility::Private,
        f,
        11,
    ));
    let s2 = g.add_symbol(make_symbol(
        "_private_util",
        SymbolKind::Function,
        Visibility::Private,
        f,
        15,
    ));
    let s3 = g.add_symbol(make_symbol(
        "active_function",
        SymbolKind::Function,
        Visibility::Private,
        f,
        18,
    ));
    let s4 = g.add_symbol(make_entry_point(
        "main_handler",
        SymbolKind::Function,
        Visibility::Public,
        f,
        22,
    ));

    // main_handler calls active_function
    add_ref(&mut g, s4, s3, ReferenceKind::Call, f);

    let _ = (s1, s2); // Explicitly unused — they're dead ends
    g
}

/// unused_import.py: os (phantom), OrderedDict (phantom), Path (used),
/// sys (used via prefix), Optional (used in annotation).
pub fn build_unused_import_graph() -> Graph {
    let mut g = Graph::new();
    let f = "unused_import.py";

    let i1 = g.add_symbol(make_import("os", f, 8));
    let i2 = g.add_symbol(make_import("OrderedDict", f, 9));
    let i3 = g.add_symbol(make_import("Path", f, 10));
    let i4 = g.add_symbol(make_import("sys", f, 11));
    let i5 = g.add_symbol(make_import("Optional", f, 12));

    let f1 = g.add_symbol(make_symbol(
        "get_args",
        SymbolKind::Function,
        Visibility::Private,
        f,
        14,
    ));
    let f2 = g.add_symbol(make_symbol(
        "resolve_path",
        SymbolKind::Function,
        Visibility::Private,
        f,
        18,
    ));
    let _f3 = g.add_symbol(make_symbol(
        "process",
        SymbolKind::Function,
        Visibility::Private,
        f,
        24,
    ));

    // get_args uses sys (sys.argv)
    add_ref(&mut g, f1, i4, ReferenceKind::Read, f);
    // resolve_path uses Path
    add_ref(&mut g, f2, i3, ReferenceKind::Call, f);
    // resolve_path uses Optional (type annotation)
    add_ref(&mut g, f2, i5, ReferenceKind::Read, f);

    let _ = (i1, i2); // Phantoms — never referenced
    g
}

/// isolated_module.py: Processor, Processor.run, process, validate form
/// an isolated cluster with internal edges but zero external inbound.
pub fn build_isolated_module_graph() -> Graph {
    let mut g = Graph::new();
    let f = "isolated_module.py";

    let c1 = g.add_symbol(make_symbol(
        "Processor",
        SymbolKind::Class,
        Visibility::Private,
        f,
        12,
    ));
    let m1 = g.add_symbol(make_symbol(
        "run",
        SymbolKind::Method,
        Visibility::Public,
        f,
        17,
    ));
    let f1 = g.add_symbol(make_symbol(
        "process",
        SymbolKind::Function,
        Visibility::Private,
        f,
        20,
    ));
    let f2 = g.add_symbol(make_symbol(
        "validate",
        SymbolKind::Function,
        Visibility::Private,
        f,
        24,
    ));

    // Processor contains run
    add_ref(&mut g, c1, m1, ReferenceKind::Read, f);
    // run calls process
    add_ref(&mut g, m1, f1, ReferenceKind::Call, f);
    // process calls validate
    add_ref(&mut g, f1, f2, ReferenceKind::Call, f);

    g
}

/// clean_code.py: all functions connected, all imports used, zero diagnostics.
pub fn build_clean_code_graph() -> Graph {
    let mut g = Graph::new();
    let f = "clean_code.py";

    let i1 = g.add_symbol(make_import("Path", f, 5));
    let f1 = g.add_symbol(make_symbol(
        "read_file",
        SymbolKind::Function,
        Visibility::Private,
        f,
        7,
    ));
    let f2 = g.add_symbol(make_symbol(
        "transform_data",
        SymbolKind::Function,
        Visibility::Private,
        f,
        11,
    ));
    let f3 = g.add_symbol(make_entry_point(
        "main",
        SymbolKind::Function,
        Visibility::Public,
        f,
        15,
    ));

    // read_file uses Path import
    add_ref(&mut g, f1, i1, ReferenceKind::Read, f);
    // transform_data calls read_file
    add_ref(&mut g, f2, f1, ReferenceKind::Call, f);
    // main calls transform_data
    add_ref(&mut g, f3, f2, ReferenceKind::Call, f);

    g
}

/// test_module.py + dead_code.py: test functions that call production code.
/// Test module must NOT be flagged as isolated cluster.
/// Test functions must NOT be flagged as dead ends.
pub fn build_test_module_graph() -> Graph {
    let mut g = Graph::new();
    let prod = "dead_code.py";
    let test = "test_module.py";

    // Production code
    let _s1 = g.add_symbol(make_symbol(
        "unused_helper",
        SymbolKind::Function,
        Visibility::Private,
        prod,
        11,
    ));
    let _s2 = g.add_symbol(make_symbol(
        "_private_util",
        SymbolKind::Function,
        Visibility::Private,
        prod,
        15,
    ));
    let s3 = g.add_symbol(make_symbol(
        "active_function",
        SymbolKind::Function,
        Visibility::Private,
        prod,
        18,
    ));
    let s4 = g.add_symbol(make_entry_point(
        "main_handler",
        SymbolKind::Function,
        Visibility::Public,
        prod,
        22,
    ));

    add_ref(&mut g, s4, s3, ReferenceKind::Call, prod);

    // Test code
    let t1 = g.add_symbol(make_symbol(
        "test_active_function",
        SymbolKind::Function,
        Visibility::Private,
        test,
        9,
    ));
    let t2 = g.add_symbol(make_symbol(
        "test_main_handler",
        SymbolKind::Function,
        Visibility::Private,
        test,
        12,
    ));

    // Tests call production code
    add_ref(&mut g, t1, s3, ReferenceKind::Call, test);
    add_ref(&mut g, t2, s4, ReferenceKind::Call, test);

    g
}

/// public_api.py: public functions with zero internal callers.
/// format_timestamp and parse_duration: LOW confidence dead ends.
/// _internal_helper: HIGH confidence dead end.
pub fn build_public_api_graph() -> Graph {
    let mut g = Graph::new();
    let f = "public_api.py";

    let _f1 = g.add_symbol(make_symbol(
        "format_timestamp",
        SymbolKind::Function,
        Visibility::Public,
        f,
        7,
    ));
    let _f2 = g.add_symbol(make_symbol(
        "parse_duration",
        SymbolKind::Function,
        Visibility::Public,
        f,
        11,
    ));
    let _f3 = g.add_symbol(make_symbol(
        "_internal_helper",
        SymbolKind::Function,
        Visibility::Private,
        f,
        16,
    ));

    // No edges — all are dead ends with different confidence
    g
}

/// side_effect_import.py: logging (used via prefix), json (phantom).
pub fn build_side_effect_import_graph() -> Graph {
    let mut g = Graph::new();
    let f = "side_effect_import.py";

    let m1 = g.add_symbol(make_symbol(
        "side_effect_import",
        SymbolKind::Module,
        Visibility::Public,
        f,
        1,
    ));
    let i1 = g.add_symbol(make_import("logging", f, 7));
    let i2 = g.add_symbol(make_import("json", f, 8));
    let _f1 = g.add_symbol(make_symbol(
        "do_work",
        SymbolKind::Function,
        Visibility::Private,
        f,
        12,
    ));

    // Module-level code references logging (logging.basicConfig)
    add_ref(&mut g, m1, i1, ReferenceKind::Read, f);

    let _ = i2; // json is phantom — never referenced
    g
}

/// reexport_init.py: helper (re-exported, NOT phantom), internal_only (phantom).
pub fn build_reexport_init_graph() -> Graph {
    let mut g = Graph::new();
    let f = "reexport_init.py";

    let m1 = g.add_symbol(make_symbol(
        "reexport_init",
        SymbolKind::Module,
        Visibility::Public,
        f,
        1,
    ));
    let i1 = g.add_symbol(make_import("helper", f, 10));
    let i2 = g.add_symbol(make_import("internal_only", f, 11));

    // Module re-exports helper (via __all__)
    add_ref(&mut g, m1, i1, ReferenceKind::Read, f);

    let _ = i2; // internal_only is neither used nor re-exported
    g
}

/// Single orphan function — should NOT trigger isolated_cluster.
pub fn build_single_orphan_graph() -> Graph {
    let mut g = Graph::new();
    let _f1 = g.add_symbol(make_symbol(
        "lonely_function",
        SymbolKind::Function,
        Visibility::Private,
        "orphan.py",
        1,
    ));
    g
}

/// Re-export-only module — only imports, no logic. Should NOT be isolated_cluster.
pub fn build_reexport_only_module_graph() -> Graph {
    let mut g = Graph::new();
    let f = "init.py";

    let m = g.add_symbol(make_symbol(
        "init",
        SymbolKind::Module,
        Visibility::Public,
        f,
        1,
    ));
    let i1 = g.add_symbol(make_import("helper_a", f, 2));
    let i2 = g.add_symbol(make_import("helper_b", f, 3));

    // Module re-exports both
    add_ref(&mut g, m, i1, ReferenceKind::Read, f);
    add_ref(&mut g, m, i2, ReferenceKind::Read, f);

    g
}

/// Isolated cluster that also contains a dead-end function.
/// A calls B, B calls C, D is in the cluster but never called internally.
pub fn build_isolated_cluster_with_dead_end_graph() -> Graph {
    let mut g = Graph::new();
    let f = "mixed.py";

    let a = g.add_symbol(make_symbol(
        "entry_fn",
        SymbolKind::Function,
        Visibility::Private,
        f,
        1,
    ));
    let b = g.add_symbol(make_symbol(
        "worker_fn",
        SymbolKind::Function,
        Visibility::Private,
        f,
        5,
    ));
    let c = g.add_symbol(make_symbol(
        "helper_fn",
        SymbolKind::Function,
        Visibility::Private,
        f,
        9,
    ));
    let d = g.add_symbol(make_symbol(
        "dead_in_cluster",
        SymbolKind::Function,
        Visibility::Private,
        f,
        13,
    ));

    // A -> B -> C (chain)
    add_ref(&mut g, a, b, ReferenceKind::Call, f);
    add_ref(&mut g, b, c, ReferenceKind::Call, f);
    // D is connected to A but nothing calls D
    add_ref(&mut g, d, a, ReferenceKind::Call, f);

    g
}

/// Build a graph from ALL fixture scenarios combined.
pub fn build_all_fixtures_graph() -> Graph {
    let mut g = Graph::new();

    // --- dead_code.py ---
    let dc = "dead_code.py";
    let _s1 = g.add_symbol(make_symbol(
        "unused_helper",
        SymbolKind::Function,
        Visibility::Private,
        dc,
        11,
    ));
    let _s2 = g.add_symbol(make_symbol(
        "_private_util",
        SymbolKind::Function,
        Visibility::Private,
        dc,
        15,
    ));
    let s3 = g.add_symbol(make_symbol(
        "active_function",
        SymbolKind::Function,
        Visibility::Private,
        dc,
        18,
    ));
    let s4 = g.add_symbol(make_entry_point(
        "main_handler",
        SymbolKind::Function,
        Visibility::Public,
        dc,
        22,
    ));
    add_ref(&mut g, s4, s3, ReferenceKind::Call, dc);

    // --- isolated_module.py ---
    let im = "isolated_module.py";
    let c1 = g.add_symbol(make_symbol(
        "Processor",
        SymbolKind::Class,
        Visibility::Private,
        im,
        12,
    ));
    let m1 = g.add_symbol(make_symbol(
        "run",
        SymbolKind::Method,
        Visibility::Public,
        im,
        17,
    ));
    let f1 = g.add_symbol(make_symbol(
        "process",
        SymbolKind::Function,
        Visibility::Private,
        im,
        20,
    ));
    let f2 = g.add_symbol(make_symbol(
        "validate",
        SymbolKind::Function,
        Visibility::Private,
        im,
        24,
    ));
    add_ref(&mut g, c1, m1, ReferenceKind::Read, im);
    add_ref(&mut g, m1, f1, ReferenceKind::Call, im);
    add_ref(&mut g, f1, f2, ReferenceKind::Call, im);

    // --- unused_import.py ---
    let ui = "unused_import.py";
    let i1 = g.add_symbol(make_import("os", ui, 8));
    let i2 = g.add_symbol(make_import("OrderedDict", ui, 9));
    let i3 = g.add_symbol(make_import("Path", ui, 10));
    let i4 = g.add_symbol(make_import("sys", ui, 11));
    let i5 = g.add_symbol(make_import("Optional", ui, 12));
    let uf1 = g.add_symbol(make_symbol(
        "get_args",
        SymbolKind::Function,
        Visibility::Private,
        ui,
        14,
    ));
    let uf2 = g.add_symbol(make_symbol(
        "resolve_path",
        SymbolKind::Function,
        Visibility::Private,
        ui,
        18,
    ));
    let _uf3 = g.add_symbol(make_symbol(
        "process",
        SymbolKind::Function,
        Visibility::Private,
        ui,
        24,
    ));
    add_ref(&mut g, uf1, i4, ReferenceKind::Read, ui);
    add_ref(&mut g, uf2, i3, ReferenceKind::Call, ui);
    add_ref(&mut g, uf2, i5, ReferenceKind::Read, ui);
    let _ = (i1, i2);

    // --- clean_code.py ---
    let cc = "clean_code.py";
    let ci = g.add_symbol(make_import("Path", cc, 5));
    let cf1 = g.add_symbol(make_symbol(
        "read_file",
        SymbolKind::Function,
        Visibility::Private,
        cc,
        7,
    ));
    let cf2 = g.add_symbol(make_symbol(
        "transform_data",
        SymbolKind::Function,
        Visibility::Private,
        cc,
        11,
    ));
    let cf3 = g.add_symbol(make_entry_point(
        "main",
        SymbolKind::Function,
        Visibility::Public,
        cc,
        15,
    ));
    add_ref(&mut g, cf1, ci, ReferenceKind::Read, cc);
    add_ref(&mut g, cf2, cf1, ReferenceKind::Call, cc);
    add_ref(&mut g, cf3, cf2, ReferenceKind::Call, cc);

    g
}
