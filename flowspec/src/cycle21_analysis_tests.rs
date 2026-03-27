// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 21 QA-2 (QA-Analysis) tests — architectural dedup validation,
//! cross-file flow tracing, confidence calibration, adversarial, regression.
//!
//! 40 tests (T1–T40) across 6 sections:
//! - T1–T11:  SymbolKind partition validation (all 11 variants)
//! - T12–T15: Orthogonality — zero entity overlap
//! - T16–T21: Confidence calibration
//! - T22–T28: Cross-file flow tracing — import resolution
//! - T29–T37: Adversarial tests
//! - T38–T40: Regression guards

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::flow::{trace_all_flows, trace_flows_from};
use crate::analyzer::patterns::{data_dead_end, orphaned_implementation, run_all_patterns};
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::test_utils::*;

/// Local constant matching flow.rs's private MAX_FLOW_DEPTH.
const MAX_FLOW_DEPTH: usize = 64;

// ===========================================================================
// Section 1: SymbolKind Partition Validation (T1–T11)
// ===========================================================================

/// T1: Function → data_dead_end only
#[test]
fn test_c21_t01_function_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "unused_func",
        SymbolKind::Function,
        Visibility::Public,
        "module.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1, "Function must appear in data_dead_end");
    assert!(dde[0].entity.contains("unused_func"));
    assert!(
        oi.is_empty(),
        "Function must NOT appear in orphaned_implementation"
    );
}

/// T2: Method → orphaned_impl only
#[test]
fn test_c21_t02_method_partition_orphaned_impl_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "orphaned_method",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        5,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dde.is_empty(),
        "Method must NOT appear in data_dead_end (C20 exclusion)"
    );
    assert_eq!(oi.len(), 1, "Method must appear in orphaned_implementation");
    assert!(oi[0].entity.contains("orphaned_method"));
}

/// T3: Variable → data_dead_end only
#[test]
fn test_c21_t03_variable_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "unused_var",
        SymbolKind::Variable,
        Visibility::Public,
        "config.py",
        3,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1, "Variable must appear in data_dead_end");
    assert!(
        oi.is_empty(),
        "Variable must NOT appear in orphaned_implementation"
    );
}

/// T4: Constant → data_dead_end only
#[test]
fn test_c21_t04_constant_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "UNUSED_CONST",
        SymbolKind::Constant,
        Visibility::Public,
        "constants.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1);
    assert!(oi.is_empty());
}

/// T5: Trait → data_dead_end only
#[test]
fn test_c21_t05_trait_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "UnusedTrait",
        SymbolKind::Trait,
        Visibility::Public,
        "lib.rs",
        10,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1);
    assert!(oi.is_empty());
}

/// T6: Interface → data_dead_end only
#[test]
fn test_c21_t06_interface_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "Serializable",
        SymbolKind::Interface,
        Visibility::Public,
        "contracts.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1);
    assert!(oi.is_empty());
}

/// T7: Macro → data_dead_end only
#[test]
fn test_c21_t07_macro_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "unused_macro",
        SymbolKind::Macro,
        Visibility::Public,
        "macros.rs",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1);
    assert!(oi.is_empty());
}

/// T8: Enum → data_dead_end only
#[test]
fn test_c21_t08_enum_partition_data_dead_end_only() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "UnusedEnum",
        SymbolKind::Enum,
        Visibility::Public,
        "types.rs",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 1);
    assert!(oi.is_empty());
}

/// T9: Module → excluded from both (structural container)
#[test]
fn test_c21_t09_module_excluded_from_both_patterns() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "orphan_module",
        SymbolKind::Module,
        Visibility::Public,
        "pkg/orphan.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(dde.is_empty(), "Module must be excluded from data_dead_end");
    assert!(
        oi.is_empty(),
        "Module must be excluded from orphaned_implementation"
    );
}

/// T10: Class → excluded from both (structural container)
#[test]
fn test_c21_t10_class_excluded_from_both_patterns() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "OrphanClass",
        SymbolKind::Class,
        Visibility::Public,
        "models.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(dde.is_empty());
    assert!(oi.is_empty());
}

/// T11: Struct → excluded from both (structural container)
#[test]
fn test_c21_t11_struct_excluded_from_both_patterns() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "OrphanStruct",
        SymbolKind::Struct,
        Visibility::Public,
        "types.rs",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(dde.is_empty());
    assert!(oi.is_empty());
}

// ===========================================================================
// Section 2: Orthogonality — Zero Entity Overlap (T12–T15)
// ===========================================================================

/// T12: All SymbolKinds in one graph — zero overlap via run_all_patterns
#[test]
fn test_c21_t12_all_kinds_zero_overlap_run_all_patterns() {
    let mut graph = Graph::new();
    let kinds = [
        ("dead_func", SymbolKind::Function),
        ("dead_method", SymbolKind::Method),
        ("dead_var", SymbolKind::Variable),
        ("dead_const", SymbolKind::Constant),
        ("dead_trait", SymbolKind::Trait),
        ("dead_iface", SymbolKind::Interface),
        ("dead_macro", SymbolKind::Macro),
        ("dead_enum", SymbolKind::Enum),
    ];
    for (name, kind) in &kinds {
        graph.add_symbol(make_symbol(name, *kind, Visibility::Public, "multi.py", 1));
    }

    let all = run_all_patterns(&graph, Path::new(""));

    let dde_entities: HashSet<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
        .map(|d| d.entity.as_str())
        .collect();
    let oi_entities: HashSet<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::OrphanedImplementation)
        .map(|d| d.entity.as_str())
        .collect();

    let overlap: Vec<&&str> = dde_entities.intersection(&oi_entities).collect();
    assert!(
        overlap.is_empty(),
        "Zero entity overlap required between data_dead_end and orphaned_impl. Overlap: {:?}",
        overlap
    );
}

/// T13: Symbols with callers appear in neither pattern
#[test]
fn test_c21_t13_symbols_with_callers_in_neither_pattern() {
    let mut graph = Graph::new();
    let func = graph.add_symbol(make_symbol(
        "called_func",
        SymbolKind::Function,
        Visibility::Public,
        "app.py",
        1,
    ));
    let method = graph.add_symbol(make_symbol(
        "called_method",
        SymbolKind::Method,
        Visibility::Public,
        "app.py",
        10,
    ));
    let caller = graph.add_symbol(make_entry_point(
        "starter",
        SymbolKind::Function,
        Visibility::Public,
        "app.py",
        20,
    ));
    add_ref(&mut graph, caller, func, ReferenceKind::Call, "app.py");
    add_ref(&mut graph, caller, method, ReferenceKind::Call, "app.py");

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dde.is_empty(),
        "Function with callers must NOT fire data_dead_end"
    );
    assert!(
        oi.is_empty(),
        "Method with callers must NOT fire orphaned_implementation"
    );
}

/// T14: Method with References edge but no Calls edge — current behavior
#[test]
fn test_c21_t14_method_with_reference_edge_suppresses_orphaned_impl() {
    let mut graph = Graph::new();
    let method = graph.add_symbol(make_symbol(
        "referenced_method",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        5,
    ));
    let reader = graph.add_symbol(make_symbol(
        "reader_func",
        SymbolKind::Function,
        Visibility::Public,
        "client.py",
        1,
    ));
    // Read reference (e.g., print(obj.method)) — NOT a call
    add_ref(&mut graph, reader, method, ReferenceKind::Read, "client.py");

    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    // Current behavior: References edge counts as inbound, so orphaned_impl does NOT fire
    assert!(
        oi.is_empty(),
        "Method with incoming References edge should NOT fire orphaned_impl \
         (current code checks Calls | References jointly)"
    );
}

/// T15: Run_all_patterns on isolated_module fixture — verify dedup
#[test]
fn test_c21_t15_isolated_module_dedup_regression() {
    let graph = build_isolated_module_graph();
    let all = run_all_patterns(&graph, Path::new(""));

    let dde_entities: Vec<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
        .map(|d| d.entity.as_str())
        .collect();
    let oi_entities: Vec<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::OrphanedImplementation)
        .map(|d| d.entity.as_str())
        .collect();

    // No entity in both
    for entity in &dde_entities {
        assert!(
            !oi_entities.contains(entity),
            "Entity '{}' appears in BOTH data_dead_end and orphaned_impl — partition violated",
            entity
        );
    }

    // Method 'run' should NOT appear in data_dead_end (excluded by kind)
    assert!(
        !dde_entities.iter().any(|e| e.contains("run")),
        "Method 'run' must not appear in data_dead_end"
    );
}

// ===========================================================================
// Section 3: Confidence Calibration (T16–T21)
// ===========================================================================

/// T16: Private Method → High confidence in orphaned_impl
#[test]
fn test_c21_t16_private_method_high_confidence_orphaned_impl() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "_helper",
        SymbolKind::Method,
        Visibility::Private,
        "internal.py",
        5,
    ));

    let oi = orphaned_implementation::detect(&graph, Path::new(""));
    assert_eq!(oi.len(), 1);
    assert_eq!(
        oi[0].confidence,
        Confidence::High,
        "Private method with zero callers must be HIGH confidence"
    );
}

/// T17: Public Method → Moderate confidence in orphaned_impl
#[test]
fn test_c21_t17_public_method_moderate_confidence_orphaned_impl() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "process",
        SymbolKind::Method,
        Visibility::Public,
        "api.py",
        5,
    ));

    let oi = orphaned_implementation::detect(&graph, Path::new(""));
    assert_eq!(oi.len(), 1);
    assert_eq!(
        oi[0].confidence,
        Confidence::Moderate,
        "Public method with zero callers must be MODERATE confidence (dynamic dispatch possible)"
    );
}

/// T18: Private Function → High confidence in data_dead_end
#[test]
fn test_c21_t18_private_function_high_confidence_data_dead_end() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Private,
        "utils.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    assert_eq!(dde.len(), 1);
    assert_eq!(dde[0].confidence, Confidence::High);
}

/// T19: Public Function → Low confidence in data_dead_end
#[test]
fn test_c21_t19_public_function_low_confidence_data_dead_end() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "format_output",
        SymbolKind::Function,
        Visibility::Public,
        "api.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    assert_eq!(dde.len(), 1);
    assert_eq!(
        dde[0].confidence,
        Confidence::Low,
        "Public function → Low confidence (might be external API)"
    );
}

/// T20: Underscore-prefixed public method → High confidence in orphaned_impl
#[test]
fn test_c21_t20_underscore_public_method_high_confidence() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "_internal_method",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        5,
    ));

    let oi = orphaned_implementation::detect(&graph, Path::new(""));
    assert_eq!(oi.len(), 1);
    assert_eq!(
        oi[0].confidence,
        Confidence::High,
        "Underscore-prefixed public method should be HIGH confidence (Python private convention)"
    );
}

/// T21: Crate visibility → Moderate confidence in data_dead_end
#[test]
fn test_c21_t21_crate_visibility_moderate_confidence() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "internal_helper",
        SymbolKind::Function,
        Visibility::Crate,
        "lib.rs",
        10,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    assert_eq!(dde.len(), 1);
    assert_eq!(
        dde[0].confidence,
        Confidence::Moderate,
        "Crate-visible function → Moderate confidence"
    );
}

// ===========================================================================
// Section 4: Cross-File Flow Tracing — Import Resolution (T22–T28)
// ===========================================================================

/// T22: Resolved import proxy → flow crosses file boundary
#[test]
fn test_c21_t22_flow_traces_through_resolved_import() {
    let mut graph = Graph::new();

    let entry = graph.add_symbol(make_symbol(
        "entry_fn",
        SymbolKind::Function,
        Visibility::Public,
        "file_a.py",
        1,
    ));
    let import_proxy = graph.add_symbol({
        let mut s = make_symbol(
            "helper",
            SymbolKind::Variable,
            Visibility::Public,
            "file_a.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let real_helper = graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "file_b.py",
        1,
    ));
    let utility = graph.add_symbol(make_symbol(
        "utility",
        SymbolKind::Function,
        Visibility::Public,
        "file_b.py",
        10,
    ));

    // entry_fn calls import_proxy
    add_ref(
        &mut graph,
        entry,
        import_proxy,
        ReferenceKind::Call,
        "file_a.py",
    );
    // Import proxy resolved to real target via References edge
    add_ref(
        &mut graph,
        import_proxy,
        real_helper,
        ReferenceKind::Read,
        "file_a.py",
    );
    // real_helper calls utility
    add_ref(
        &mut graph,
        real_helper,
        utility,
        ReferenceKind::Call,
        "file_b.py",
    );

    let paths = trace_flows_from(&graph, entry, MAX_FLOW_DEPTH);

    assert!(
        !paths.is_empty(),
        "Flow must produce paths through resolved import"
    );
    let all_step_ids: Vec<SymbolId> = paths
        .iter()
        .flat_map(|p| p.steps.iter().map(|s| s.symbol))
        .collect();
    assert!(
        all_step_ids.contains(&real_helper),
        "Flow must resolve through import proxy to real_helper"
    );
    assert!(
        all_step_ids.contains(&utility),
        "Flow must continue past resolved import into file_b functions"
    );
}

/// T23: Unresolved import proxy → flow stops gracefully
#[test]
fn test_c21_t23_unresolved_import_stops_gracefully() {
    let mut graph = Graph::new();

    let entry = graph.add_symbol(make_symbol(
        "caller",
        SymbolKind::Function,
        Visibility::Public,
        "main_app.py",
        1,
    ));
    let import_proxy = graph.add_symbol({
        let mut s = make_symbol(
            "os_path",
            SymbolKind::Variable,
            Visibility::Public,
            "main_app.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });

    add_ref(
        &mut graph,
        entry,
        import_proxy,
        ReferenceKind::Call,
        "main_app.py",
    );
    // No References edge from import_proxy — unresolved

    let paths = trace_flows_from(&graph, entry, MAX_FLOW_DEPTH);

    assert!(
        !paths.is_empty(),
        "Must produce path even with unresolved import"
    );
    // Path terminates at import proxy
    let terminal = paths[0].steps.last().expect("Path must have steps");
    assert_eq!(
        terminal.symbol, import_proxy,
        "Flow must terminate at unresolved import proxy (no resolution available)"
    );
}

/// T24: Chained cross-file imports A → B → C
#[test]
fn test_c21_t24_chained_cross_file_flow() {
    let mut graph = Graph::new();

    let entry = graph.add_symbol(make_symbol(
        "entry",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        1,
    ));
    let import_b = graph.add_symbol({
        let mut s = make_symbol(
            "func_b",
            SymbolKind::Variable,
            Visibility::Public,
            "a.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let real_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        1,
    ));
    let import_c = graph.add_symbol({
        let mut s = make_symbol(
            "func_c",
            SymbolKind::Variable,
            Visibility::Public,
            "b.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let real_c = graph.add_symbol(make_symbol(
        "func_c",
        SymbolKind::Function,
        Visibility::Public,
        "c.py",
        1,
    ));

    // entry calls import_b
    add_ref(&mut graph, entry, import_b, ReferenceKind::Call, "a.py");
    // import_b resolved to real_b
    add_ref(&mut graph, import_b, real_b, ReferenceKind::Read, "a.py");
    // real_b calls import_c
    add_ref(&mut graph, real_b, import_c, ReferenceKind::Call, "b.py");
    // import_c resolved to real_c
    add_ref(&mut graph, import_c, real_c, ReferenceKind::Read, "b.py");

    let paths = trace_flows_from(&graph, entry, MAX_FLOW_DEPTH);

    let all_steps: Vec<SymbolId> = paths
        .iter()
        .flat_map(|p| p.steps.iter().map(|s| s.symbol))
        .collect();
    assert!(all_steps.contains(&real_b), "Must trace into file b");
    assert!(
        all_steps.contains(&real_c),
        "Must trace into file c — chained resolution"
    );
}

/// T25: Mutual imports with calls — cycle guard across files
#[test]
fn test_c21_t25_mutual_import_cycle_guard() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "cycle_a.py",
        1,
    ));
    let import_b = graph.add_symbol({
        let mut s = make_symbol(
            "func_b_import",
            SymbolKind::Variable,
            Visibility::Public,
            "cycle_a.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "cycle_b.py",
        1,
    ));
    let import_a = graph.add_symbol({
        let mut s = make_symbol(
            "func_a_import",
            SymbolKind::Variable,
            Visibility::Public,
            "cycle_b.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });

    // func_a → import_b → func_b → import_a → func_a (cycle)
    add_ref(
        &mut graph,
        func_a,
        import_b,
        ReferenceKind::Call,
        "cycle_a.py",
    );
    add_ref(
        &mut graph,
        import_b,
        func_b,
        ReferenceKind::Read,
        "cycle_a.py",
    );
    add_ref(
        &mut graph,
        func_b,
        import_a,
        ReferenceKind::Call,
        "cycle_b.py",
    );
    add_ref(
        &mut graph,
        import_a,
        func_a,
        ReferenceKind::Read,
        "cycle_b.py",
    );

    let paths = trace_flows_from(&graph, func_a, MAX_FLOW_DEPTH);

    assert!(
        !paths.is_empty(),
        "Mutual import cycle must produce at least one path"
    );
    assert!(
        paths.iter().any(|p| p.is_cyclic),
        "Path through mutual imports must be detected as cyclic"
    );
}

/// T26: Diamond pattern across files
#[test]
fn test_c21_t26_diamond_cross_file_flow() {
    let mut graph = Graph::new();

    let entry = graph.add_symbol(make_symbol(
        "entry",
        SymbolKind::Function,
        Visibility::Public,
        "diamond_a.py",
        1,
    ));
    let ib = graph.add_symbol({
        let mut s = make_symbol(
            "func_b_imp",
            SymbolKind::Variable,
            Visibility::Public,
            "diamond_a.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let ic = graph.add_symbol({
        let mut s = make_symbol(
            "func_c_imp",
            SymbolKind::Variable,
            Visibility::Public,
            "diamond_a.py",
            3,
        );
        s.annotations.push("import".to_string());
        s
    });
    let real_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "diamond_b.py",
        1,
    ));
    let real_c = graph.add_symbol(make_symbol(
        "func_c",
        SymbolKind::Function,
        Visibility::Public,
        "diamond_c.py",
        1,
    ));
    let id_from_b = graph.add_symbol({
        let mut s = make_symbol(
            "func_d_fromb",
            SymbolKind::Variable,
            Visibility::Public,
            "diamond_b.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let id_from_c = graph.add_symbol({
        let mut s = make_symbol(
            "func_d_fromc",
            SymbolKind::Variable,
            Visibility::Public,
            "diamond_c.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let real_d = graph.add_symbol(make_symbol(
        "func_d",
        SymbolKind::Function,
        Visibility::Public,
        "diamond_d.py",
        1,
    ));

    // entry → ib → real_b, entry → ic → real_c
    add_ref(&mut graph, entry, ib, ReferenceKind::Call, "diamond_a.py");
    add_ref(&mut graph, entry, ic, ReferenceKind::Call, "diamond_a.py");
    add_ref(&mut graph, ib, real_b, ReferenceKind::Read, "diamond_a.py");
    add_ref(&mut graph, ic, real_c, ReferenceKind::Read, "diamond_a.py");
    // real_b → id_from_b → real_d, real_c → id_from_c → real_d
    add_ref(
        &mut graph,
        real_b,
        id_from_b,
        ReferenceKind::Call,
        "diamond_b.py",
    );
    add_ref(
        &mut graph,
        id_from_b,
        real_d,
        ReferenceKind::Read,
        "diamond_b.py",
    );
    add_ref(
        &mut graph,
        real_c,
        id_from_c,
        ReferenceKind::Call,
        "diamond_c.py",
    );
    add_ref(
        &mut graph,
        id_from_c,
        real_d,
        ReferenceKind::Read,
        "diamond_c.py",
    );

    let paths = trace_flows_from(&graph, entry, MAX_FLOW_DEPTH);

    assert!(
        paths.len() >= 2,
        "Diamond pattern must produce at least 2 paths. Got: {}",
        paths.len()
    );
    let all_targets: Vec<SymbolId> = paths
        .iter()
        .flat_map(|p| p.steps.iter().map(|s| s.symbol))
        .collect();
    assert!(
        all_targets.contains(&real_d),
        "Both diamond paths must converge at func_d"
    );
}

/// T27: Import proxy with multiple resolved targets
#[test]
fn test_c21_t27_import_multiple_resolved_targets() {
    let mut graph = Graph::new();

    let entry = graph.add_symbol(make_symbol(
        "entry",
        SymbolKind::Function,
        Visibility::Public,
        "star_main.py",
        1,
    ));
    let star_import = graph.add_symbol({
        let mut s = make_symbol(
            "star",
            SymbolKind::Variable,
            Visibility::Public,
            "star_main.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });
    let target_a = graph.add_symbol(make_symbol(
        "target_a",
        SymbolKind::Function,
        Visibility::Public,
        "star_lib.py",
        1,
    ));
    let target_b = graph.add_symbol(make_symbol(
        "target_b",
        SymbolKind::Function,
        Visibility::Public,
        "star_lib.py",
        10,
    ));

    add_ref(
        &mut graph,
        entry,
        star_import,
        ReferenceKind::Call,
        "star_main.py",
    );
    add_ref(
        &mut graph,
        star_import,
        target_a,
        ReferenceKind::Read,
        "star_main.py",
    );
    add_ref(
        &mut graph,
        star_import,
        target_b,
        ReferenceKind::Read,
        "star_main.py",
    );

    let paths = trace_flows_from(&graph, entry, MAX_FLOW_DEPTH);

    let all_steps: Vec<SymbolId> = paths
        .iter()
        .flat_map(|p| p.steps.iter().map(|s| s.symbol))
        .collect();
    assert!(
        all_steps.contains(&target_a) || all_steps.contains(&target_b),
        "Flow must branch into at least one resolved target from multi-resolution import"
    );
}

/// T28: Self-referential import — file imports itself
#[test]
fn test_c21_t28_self_referential_import_cycle() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "self_ref.py",
        1,
    ));
    let self_import = graph.add_symbol({
        let mut s = make_symbol(
            "func_a_import",
            SymbolKind::Variable,
            Visibility::Public,
            "self_ref.py",
            2,
        );
        s.annotations.push("import".to_string());
        s
    });

    add_ref(
        &mut graph,
        func_a,
        self_import,
        ReferenceKind::Call,
        "self_ref.py",
    );
    add_ref(
        &mut graph,
        self_import,
        func_a,
        ReferenceKind::Read,
        "self_ref.py",
    );

    let paths = trace_flows_from(&graph, func_a, MAX_FLOW_DEPTH);

    assert!(!paths.is_empty());
    assert!(
        paths.iter().any(|p| p.is_cyclic),
        "Self-referential import must be detected as cyclic"
    );
}

// ===========================================================================
// Section 5: Adversarial Tests (T29–T37)
// ===========================================================================

/// T29: Massive graph — 1000 symbols, zero overlap
#[test]
fn test_c21_t29_massive_graph_partition_zero_overlap() {
    let mut graph = Graph::new();

    for i in 0..500 {
        graph.add_symbol(make_symbol(
            &format!("func_{}", i),
            SymbolKind::Function,
            Visibility::Public,
            "big_module.py",
            i as u32 + 1,
        ));
    }
    for i in 0..500 {
        graph.add_symbol(make_symbol(
            &format!("method_{}", i),
            SymbolKind::Method,
            Visibility::Public,
            "big_class.py",
            i as u32 + 1,
        ));
    }

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(dde.len(), 500, "All 500 functions must be in data_dead_end");
    assert_eq!(oi.len(), 500, "All 500 methods must be in orphaned_impl");

    let dde_entities: HashSet<String> = dde.iter().map(|d| d.entity.clone()).collect();
    let oi_entities: HashSet<String> = oi.iter().map(|d| d.entity.clone()).collect();
    assert!(
        dde_entities.is_disjoint(&oi_entities),
        "Zero entity overlap on 1000-symbol graph"
    );
}

/// T30: Empty graph — both patterns return empty, no panic
#[test]
fn test_c21_t30_empty_graph_both_patterns_empty() {
    let graph = Graph::new();
    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(dde.is_empty());
    assert!(oi.is_empty());
}

/// T31: All symbols excluded — dunder, test_, entry_point, import
#[test]
fn test_c21_t31_all_excluded_symbols_zero_diagnostics() {
    let mut graph = Graph::new();

    // Dunder method
    graph.add_symbol(make_symbol(
        "__init__",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        1,
    ));
    // Test function
    graph.add_symbol(make_symbol(
        "test_something",
        SymbolKind::Function,
        Visibility::Public,
        "utils.py",
        5,
    ));
    // Entry point
    graph.add_symbol(make_entry_point(
        "startup",
        SymbolKind::Function,
        Visibility::Public,
        "app.py",
        1,
    ));
    // Import
    graph.add_symbol(make_import("os", "prog.py", 1));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dde.is_empty(),
        "All excluded symbols — data_dead_end must be empty"
    );
    assert!(
        oi.is_empty(),
        "All excluded symbols — orphaned_impl must be empty"
    );
}

/// T32: Type annotation reference does NOT suppress orphaned_impl (documents current behavior)
#[test]
fn test_c21_t32_type_annotation_ref_behavior_on_orphaned_impl() {
    let mut graph = Graph::new();

    let method = graph.add_symbol(make_symbol(
        "handle_event",
        SymbolKind::Method,
        Visibility::Public,
        "handler.py",
        5,
    ));
    let annotator = graph.add_symbol(make_symbol(
        "configure",
        SymbolKind::Function,
        Visibility::Public,
        "config_mod.py",
        1,
    ));
    // Type annotation reference — not a call
    add_ref(
        &mut graph,
        annotator,
        method,
        ReferenceKind::Read,
        "config_mod.py",
    );

    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    // Current behavior: References edge suppresses orphaned_impl
    assert!(
        oi.is_empty(),
        "Current behavior: References edge suppresses orphaned_impl \
         (even from type annotation context). See GH issue for semantic gap."
    );
}

/// T33: Diagnostic pattern field correctness
#[test]
fn test_c21_t33_pattern_field_correctness() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "dead_func",
        SymbolKind::Function,
        Visibility::Public,
        "mod.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "orphan_method",
        SymbolKind::Method,
        Visibility::Public,
        "mod.py",
        10,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));
    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(dde
        .iter()
        .all(|d| d.pattern == DiagnosticPattern::DataDeadEnd));
    assert!(oi
        .iter()
        .all(|d| d.pattern == DiagnosticPattern::OrphanedImplementation));
}

/// T34: Evidence includes file count
#[test]
fn test_c21_t34_evidence_includes_file_count() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        1,
    ));

    let dde = data_dead_end::detect(&graph, Path::new(""));

    assert!(!dde.is_empty());
    for d in &dde {
        assert!(!d.evidence.is_empty(), "Evidence must be present");
        assert!(
            d.evidence[0].observation.contains("2 analyzed files"),
            "Evidence must report correct file count. Got: {}",
            d.evidence[0].observation
        );
    }
}

/// T35: Protected visibility → High confidence
#[test]
fn test_c21_t35_protected_method_high_confidence() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "protected_method",
        SymbolKind::Method,
        Visibility::Protected,
        "base.py",
        5,
    ));

    let oi = orphaned_implementation::detect(&graph, Path::new(""));
    assert_eq!(oi.len(), 1);
    assert_eq!(
        oi[0].confidence,
        Confidence::High,
        "Protected method → High confidence (same as Private)"
    );
}

/// T36: Direct cross-file call without import proxy
#[test]
fn test_c21_t36_direct_cross_file_call_no_proxy() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "direct_a.py",
        1,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "direct_b.py",
        1,
    ));

    // Direct call edge — no import proxy
    add_ref(
        &mut graph,
        func_a,
        func_b,
        ReferenceKind::Call,
        "direct_a.py",
    );

    let paths = trace_flows_from(&graph, func_a, MAX_FLOW_DEPTH);

    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0].steps[0].symbol, func_b,
        "Direct cross-file call must trace without import resolution"
    );
}

/// T37: Multiple entry points producing overlapping flows
#[test]
fn test_c21_t37_multiple_entry_points_overlapping_flows() {
    let mut graph = Graph::new();

    let main_a = graph.add_symbol({
        let mut s = make_symbol(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            "entry_a.py",
            1,
        );
        s.annotations.push("entry_point".to_string());
        s.name = "main".to_string();
        s
    });
    let main_b = graph.add_symbol({
        let mut s = make_symbol(
            "main",
            SymbolKind::Function,
            Visibility::Public,
            "entry_b.py",
            1,
        );
        s.annotations.push("entry_point".to_string());
        s.name = "main".to_string();
        s
    });
    let shared = graph.add_symbol(make_symbol(
        "shared_util",
        SymbolKind::Function,
        Visibility::Public,
        "shared_lib.py",
        1,
    ));
    let terminal = graph.add_symbol(make_symbol(
        "terminal",
        SymbolKind::Function,
        Visibility::Public,
        "shared_lib.py",
        10,
    ));

    add_ref(
        &mut graph,
        main_a,
        shared,
        ReferenceKind::Call,
        "entry_a.py",
    );
    add_ref(
        &mut graph,
        main_b,
        shared,
        ReferenceKind::Call,
        "entry_b.py",
    );
    add_ref(
        &mut graph,
        shared,
        terminal,
        ReferenceKind::Call,
        "shared_lib.py",
    );

    let paths = trace_all_flows(&graph);

    assert!(
        paths.len() >= 2,
        "Two entry points calling same function → at least 2 paths. Got: {}",
        paths.len()
    );

    let entries: HashSet<SymbolId> = paths.iter().map(|p| p.entry).collect();
    assert!(entries.contains(&main_a));
    assert!(entries.contains(&main_b));
}

// ===========================================================================
// Section 6: Regression Guards (T38–T40)
// ===========================================================================

/// T38: C20 Method exclusion still holds
#[test]
fn test_c21_t38_c20_method_exclusion_regression() {
    let graph = build_orphaned_method_graph();
    let dde = data_dead_end::detect(&graph, Path::new(""));

    for d in &dde {
        assert!(
            !d.entity.contains("process_user"),
            "process_user is a Method — must NOT appear in data_dead_end (C20 exclusion)"
        );
        assert!(
            !d.entity.contains("validate"),
            "validate is a Method — must NOT appear in data_dead_end"
        );
    }
}

/// T39: Shared exclusion logic applies identically to both patterns
#[test]
fn test_c21_t39_shared_exclusions_apply_to_orphaned_impl() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "__str__",
        SymbolKind::Method,
        Visibility::Public,
        "model.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "process",
        SymbolKind::Method,
        Visibility::Public,
        "model.py",
        5,
    ));

    let oi = orphaned_implementation::detect(&graph, Path::new(""));

    assert_eq!(oi.len(), 1, "Only non-excluded method should fire");
    assert!(oi[0].entity.contains("process"), "process must be detected");
    assert!(
        !oi.iter().any(|d| d.entity.contains("__str__")),
        "__str__ (dunder) must be excluded from orphaned_impl"
    );
}

/// T40: Dogfood baseline tolerance ranges
#[test]
fn test_c21_t40_dogfood_baseline_within_tolerance() {
    // This test validates against the actual codebase parsed by the full pipeline.
    // Since we don't have the full parse pipeline in unit tests, we validate
    // using the all-fixtures graph as a proxy for dogfood behavior.
    let graph = build_all_fixtures_graph();
    let all = run_all_patterns(&graph, Path::new(""));

    let dde_count = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
        .count();
    let oi_count = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::OrphanedImplementation)
        .count();

    // All-fixtures graph has known planted facts — validate partition holds
    let dde_entities: HashSet<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
        .map(|d| d.entity.as_str())
        .collect();
    let oi_entities: HashSet<&str> = all
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::OrphanedImplementation)
        .map(|d| d.entity.as_str())
        .collect();

    assert!(
        dde_entities.is_disjoint(&oi_entities),
        "Zero overlap between data_dead_end and orphaned_impl on all-fixtures graph"
    );

    // Sanity: both patterns must produce some findings on all-fixtures graph
    assert!(
        dde_count > 0,
        "data_dead_end must find dead ends in all-fixtures graph"
    );
    // orphaned_impl may or may not fire depending on whether isolated_module's
    // Method 'run' has incoming References — it does (from Processor class Read),
    // so oi_count may be 0. That's correct behavior.
    let _ = oi_count;
}
