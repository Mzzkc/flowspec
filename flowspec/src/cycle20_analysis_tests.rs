// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 20 QA-2 (QA-Analysis) tests — circular dependency Python import cycles,
//! orphaned_impl/data_dead_end dedup, dogfood regression, cross-pattern orthogonality.
//!
//! 38 tests (T1–T38) across 8 sections:
//! - T1–T5:   Circular dependency — Python import cycle true positives
//! - T6–T9:   Circular dependency — true negatives
//! - T10–T13: Circular dependency — adversarial tests
//! - T14–T15: Circular dependency — resolution gap regression
//! - T16–T23: orphaned_impl / data_dead_end dedup core
//! - T24–T29: Dedup adversarial + cross-pattern orthogonality
//! - T30–T32: Dogfood baseline impact
//! - T33–T38: Regression tests

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::{
    circular_dependency, data_dead_end, orphaned_implementation, run_all_patterns,
};
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::test_utils::*;

// ===========================================================================
// Section 1: Circular Dependency — Python Import Cycle True Positives (T1–T5)
// ===========================================================================

/// T1: Two Python files importing each other. When resolution succeeds and
/// creates EdgeKind::References edges (via ReferenceKind::Import), the
/// detector must find the cycle.
#[test]
fn test_c20_t01_circular_dep_two_file_python_import_cycle() {
    let mut graph = Graph::new();

    // a.py: defines func_a, imports func_b from b.py
    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/a.py",
        5,
    ));
    let import_b = graph.add_symbol({
        let mut sym = make_import("func_b", "pkg/a.py", 1);
        sym.annotations.push("from:b".to_string());
        sym
    });

    // b.py: defines func_b, imports func_a from a.py
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/b.py",
        5,
    ));
    let import_a = graph.add_symbol({
        let mut sym = make_import("func_a", "pkg/b.py", 1);
        sym.annotations.push("from:a".to_string());
        sym
    });

    // Simulate resolved cross-file import edges
    add_ref(
        &mut graph,
        import_b,
        func_b,
        ReferenceKind::Import,
        "pkg/a.py",
    );
    add_ref(
        &mut graph,
        import_a,
        func_a,
        ReferenceKind::Import,
        "pkg/b.py",
    );

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        1,
        "Must detect exactly 1 cycle between a.py and b.py"
    );
    assert_eq!(results[0].confidence, Confidence::High);
    assert_eq!(results[0].severity, Severity::Warning);
}

/// T2: Three-file Python import cycle (A → B → C → A).
#[test]
fn test_c20_t02_circular_dep_three_file_python_cycle() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/a.py",
        5,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/b.py",
        5,
    ));
    let func_c = graph.add_symbol(make_symbol(
        "func_c",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/c.py",
        5,
    ));

    // a.py imports from b.py
    let imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "pkg/a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    // b.py imports from c.py
    let imp_bc = graph.add_symbol({
        let mut s = make_import("func_c", "pkg/b.py", 1);
        s.annotations.push("from:c".to_string());
        s
    });
    // c.py imports from a.py
    let imp_ca = graph.add_symbol({
        let mut s = make_import("func_a", "pkg/c.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });

    add_ref(
        &mut graph,
        imp_ab,
        func_b,
        ReferenceKind::Import,
        "pkg/a.py",
    );
    add_ref(
        &mut graph,
        imp_bc,
        func_c,
        ReferenceKind::Import,
        "pkg/b.py",
    );
    add_ref(
        &mut graph,
        imp_ca,
        func_a,
        ReferenceKind::Import,
        "pkg/c.py",
    );

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 1, "Must detect exactly 1 three-module cycle");

    // Verify all three modules appear in the entity
    let entity = &results[0].entity;
    assert!(entity.contains("a.py"), "Cycle entity must mention a.py");
    assert!(entity.contains("b.py"), "Cycle entity must mention b.py");
    assert!(entity.contains("c.py"), "Cycle entity must mention c.py");
}

/// T3: Four-file Python import cycle.
#[test]
fn test_c20_t03_circular_dep_four_file_python_cycle() {
    let mut graph = Graph::new();

    let files = ["pkg/w.py", "pkg/x.py", "pkg/y.py", "pkg/z.py"];
    let mut funcs = Vec::new();
    for (i, f) in files.iter().enumerate() {
        let name = format!("func_{}", (b'w' + i as u8) as char);
        funcs.push(graph.add_symbol(make_symbol(
            &name,
            SymbolKind::Function,
            Visibility::Public,
            f,
            5,
        )));
    }

    // Create cycle: w → x → y → z → w
    for i in 0..4 {
        let next = (i + 1) % 4;
        let imp = graph.add_symbol({
            let mut s = make_import(
                &format!("func_{}", (b'w' + next as u8) as char),
                files[i],
                1,
            );
            s.annotations.push(format!("from:{}", files[next]));
            s
        });
        add_ref(
            &mut graph,
            imp,
            funcs[next],
            ReferenceKind::Import,
            files[i],
        );
    }

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 1, "Must detect exactly 1 four-module cycle");
    assert!(
        results[0].evidence.len() >= 4,
        "Evidence should have 4+ steps for 4-module cycle"
    );
}

/// T4: Two independent cycles detected separately.
#[test]
fn test_c20_t04_circular_dep_two_independent_python_cycles() {
    let mut graph = Graph::new();

    // Cycle 1: a.py ↔ b.py
    let fa = graph.add_symbol(make_symbol(
        "fa",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let fb = graph.add_symbol(make_symbol(
        "fb",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));
    let i_ab = graph.add_symbol({
        let mut s = make_import("fb", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let i_ba = graph.add_symbol({
        let mut s = make_import("fa", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    add_ref(&mut graph, i_ab, fb, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, i_ba, fa, ReferenceKind::Import, "b.py");

    // Cycle 2: c.py ↔ d.py (completely separate)
    let fc = graph.add_symbol(make_symbol(
        "fc",
        SymbolKind::Function,
        Visibility::Public,
        "c.py",
        5,
    ));
    let fd = graph.add_symbol(make_symbol(
        "fd",
        SymbolKind::Function,
        Visibility::Public,
        "d.py",
        5,
    ));
    let i_cd = graph.add_symbol({
        let mut s = make_import("fd", "c.py", 1);
        s.annotations.push("from:d".to_string());
        s
    });
    let i_dc = graph.add_symbol({
        let mut s = make_import("fc", "d.py", 1);
        s.annotations.push("from:c".to_string());
        s
    });
    add_ref(&mut graph, i_cd, fd, ReferenceKind::Import, "c.py");
    add_ref(&mut graph, i_dc, fc, ReferenceKind::Import, "d.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 2, "Must detect both independent cycles");
}

/// T5: Cycle evidence contains per-step cross-module references.
#[test]
fn test_c20_t05_circular_dep_python_cycle_evidence_completeness() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        10,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        20,
    ));
    let imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let imp_ba = graph.add_symbol({
        let mut s = make_import("func_a", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    add_ref(&mut graph, imp_ab, func_b, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, imp_ba, func_a, ReferenceKind::Import, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 1);
    let diag = &results[0];

    // Evidence must have exactly 2 steps (A→B, B→A)
    assert_eq!(
        diag.evidence.len(),
        2,
        "2-module cycle must have 2 evidence steps"
    );
    // Each step must have an observation containing the cross-reference detail
    for ev in &diag.evidence {
        assert!(
            !ev.observation.is_empty(),
            "Evidence observation must be non-empty"
        );
        assert!(
            ev.location.is_some(),
            "Evidence location must be present for resolved refs"
        );
    }
}

// ===========================================================================
// Section 2: Circular Dependency — True Negatives (T6–T9)
// ===========================================================================

/// T6: Linear import chain (no back-edge) produces zero findings.
#[test]
fn test_c20_t06_circular_dep_linear_python_import_chain_no_cycle() {
    let mut graph = Graph::new();

    let _func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));
    let func_c = graph.add_symbol(make_symbol(
        "func_c",
        SymbolKind::Function,
        Visibility::Public,
        "c.py",
        5,
    ));

    // a → b → c (no back-edge)
    let imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let imp_bc = graph.add_symbol({
        let mut s = make_import("func_c", "b.py", 1);
        s.annotations.push("from:c".to_string());
        s
    });
    add_ref(&mut graph, imp_ab, func_b, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, imp_bc, func_c, ReferenceKind::Import, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        0,
        "Linear chain must NOT produce a cycle finding"
    );
}

/// T7: Intra-module imports do not create false cycle.
#[test]
fn test_c20_t07_circular_dep_intra_module_imports_no_false_positive() {
    let mut graph = Graph::new();

    // Two functions in the same file referencing each other
    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "utils.py",
        1,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "utils.py",
        10,
    ));
    add_ref(&mut graph, func_a, func_b, ReferenceKind::Call, "utils.py");
    add_ref(&mut graph, func_b, func_a, ReferenceKind::Call, "utils.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        0,
        "Same-file mutual references are not circular dependencies"
    );
}

/// T8: Diamond dependency (A→B, A→C, B→D, C→D) is not a cycle.
#[test]
fn test_c20_t08_circular_dep_diamond_python_imports_not_cycle() {
    let mut graph = Graph::new();

    let fa = graph.add_symbol(make_symbol(
        "fa",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let fb = graph.add_symbol(make_symbol(
        "fb",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));
    let fc = graph.add_symbol(make_symbol(
        "fc",
        SymbolKind::Function,
        Visibility::Public,
        "c.py",
        5,
    ));
    let fd = graph.add_symbol(make_symbol(
        "fd",
        SymbolKind::Function,
        Visibility::Public,
        "d.py",
        5,
    ));

    // A → B, A → C, B → D, C → D (diamond, no back-edge)
    let i_ab = graph.add_symbol({
        let mut s = make_import("fb", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let i_ac = graph.add_symbol({
        let mut s = make_import("fc", "a.py", 2);
        s.annotations.push("from:c".to_string());
        s
    });
    let i_bd = graph.add_symbol({
        let mut s = make_import("fd", "b.py", 1);
        s.annotations.push("from:d".to_string());
        s
    });
    let i_cd = graph.add_symbol({
        let mut s = make_import("fd", "c.py", 1);
        s.annotations.push("from:d".to_string());
        s
    });
    add_ref(&mut graph, i_ab, fb, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, i_ac, fc, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, i_bd, fd, ReferenceKind::Import, "b.py");
    add_ref(&mut graph, i_cd, fd, ReferenceKind::Import, "c.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 0, "Diamond dependency is NOT a cycle");
    let _ = (fa, fc); // suppress unused warnings
}

/// T9: Single-file Python project produces zero findings.
#[test]
fn test_c20_t09_circular_dep_single_file_python_no_cycle() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "main",
        SymbolKind::Function,
        Visibility::Public,
        "app.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Private,
        "app.py",
        10,
    ));

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        0,
        "Single-file project cannot have circular dependencies"
    );
}

// ===========================================================================
// Section 3: Circular Dependency — Adversarial Tests (T10–T13)
// ===========================================================================

/// T10: Self-import (file imports from itself) must not trigger.
#[test]
fn test_c20_t10_circular_dep_self_import_not_flagged() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let self_imp = graph.add_symbol({
        let mut s = make_import("func_a", "a.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    // Self-referencing import edge (same file)
    add_ref(&mut graph, self_imp, func_a, ReferenceKind::Import, "a.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        0,
        "Self-import (same file) must not be flagged as circular dependency"
    );
}

/// T11: Cycle with mixed edge types (Call + Import across modules).
#[test]
fn test_c20_t11_circular_dep_mixed_call_and_import_edges_form_cycle() {
    let mut graph = Graph::new();

    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));

    // a.py → b.py via Import edge
    let imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    add_ref(&mut graph, imp_ab, func_b, ReferenceKind::Import, "a.py");

    // b.py → a.py via Call edge (direct function call after import resolution)
    add_ref(&mut graph, func_b, func_a, ReferenceKind::Call, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        1,
        "Mixed Import+Call edges across modules form a cycle"
    );
}

/// T12: TYPE_CHECKING-only cycle — documents design question.
/// Type-only cycles are common in Python and harmless at runtime.
#[test]
fn test_c20_t12_circular_dep_type_checking_only_cycle_question() {
    let mut graph = Graph::new();

    // Both files import each other ONLY inside TYPE_CHECKING blocks
    let func_a = graph.add_symbol(make_symbol(
        "ClassA",
        SymbolKind::Class,
        Visibility::Public,
        "a.py",
        5,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "ClassB",
        SymbolKind::Class,
        Visibility::Public,
        "b.py",
        5,
    ));

    // a.py: if TYPE_CHECKING: from b import ClassB
    let imp_ab = graph.add_symbol({
        let mut s = make_import("ClassB", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s.annotations.push("type_checking_import".to_string());
        s
    });
    // b.py: if TYPE_CHECKING: from a import ClassA
    let imp_ba = graph.add_symbol({
        let mut s = make_import("ClassA", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s.annotations.push("type_checking_import".to_string());
        s
    });

    add_ref(&mut graph, imp_ab, func_b, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, imp_ba, func_a, ReferenceKind::Import, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    // DESIGN DECISION: If TYPE_CHECKING-aware filtering is implemented in the
    // adjacency builder, this should be 0. If not, it's 1 (acceptable for now).
    let count = results.len();
    assert!(
        count <= 1,
        "TYPE_CHECKING-only cycle should produce 0 or 1 finding (design decision pending)"
    );
}

/// T13: Re-export chain forming a cycle IS a circular dependency at module level.
#[test]
fn test_c20_t13_circular_dep_reexport_chain_is_cyclic() {
    let mut graph = Graph::new();

    // a.py exports a function, b.py re-exports it
    let func_a = graph.add_symbol(make_symbol(
        "shared_fn",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let reexport_b = graph.add_symbol({
        let mut s = make_import("shared_fn", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    add_ref(
        &mut graph,
        reexport_b,
        func_a,
        ReferenceKind::Export,
        "b.py",
    );

    // b.py has its own function used by a.py
    let func_b = graph.add_symbol(make_symbol(
        "helper_fn",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        10,
    ));
    let imp_ab = graph.add_symbol({
        let mut s = make_import("helper_fn", "a.py", 2);
        s.annotations.push("from:b".to_string());
        s
    });
    add_ref(&mut graph, imp_ab, func_b, ReferenceKind::Import, "a.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    // This IS a cycle at the module level (a→b via Import, b→a via Export)
    // Both produce EdgeKind::References. The detector should find it.
    assert_eq!(
        results.len(),
        1,
        "Re-export chain forming a cycle IS a circular dependency at module level"
    );
}

// ===========================================================================
// Section 4: Circular Dependency — Resolution Gap Regression (T14–T15)
// ===========================================================================

/// T14: Unresolved imports create no cross-module edges (documents the gap).
/// This test PASSES today — it's a gap documentation test.
#[test]
fn test_c20_t14_circular_dep_unresolved_imports_produce_zero_findings() {
    let mut graph = Graph::new();

    // Two files with import symbols but NO resolved cross-file edges
    let _func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/a.py",
        5,
    ));
    let _func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "pkg/b.py",
        5,
    ));

    // Import symbols exist but resolution never happened (no add_ref calls)
    let _imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "pkg/a.py", 1);
        s.annotations.push("from:b".to_string());
        s.resolution = ResolutionStatus::Unresolved;
        s
    });
    let _imp_ba = graph.add_symbol({
        let mut s = make_import("func_a", "pkg/b.py", 1);
        s.annotations.push("from:a".to_string());
        s.resolution = ResolutionStatus::Unresolved;
        s
    });

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        0,
        "Unresolved imports create no cross-module edges, so detector sees no cycle. \
         This documents the field test gap — 0/13 cycles found on Mozart because \
         Python relative imports were not resolved."
    );
}

/// T15: Package-structured project with resolved relative imports detects cycle.
/// Simulates the post-fix state by manually creating resolved edges.
#[test]
fn test_c20_t15_circular_dep_package_relative_imports_with_resolution() {
    let mut graph = Graph::new();

    // Package: mypackage/a.py and mypackage/b.py
    let func_a = graph.add_symbol(make_symbol(
        "func_a",
        SymbolKind::Function,
        Visibility::Public,
        "mypackage/a.py",
        5,
    ));
    let func_b = graph.add_symbol(make_symbol(
        "func_b",
        SymbolKind::Function,
        Visibility::Public,
        "mypackage/b.py",
        5,
    ));

    // Simulating: from .b import func_b (resolved)
    let imp_ab = graph.add_symbol({
        let mut s = make_import("func_b", "mypackage/a.py", 1);
        s.annotations.push("from:b".to_string()); // bare name from relative import
        s.resolution = ResolutionStatus::Resolved;
        s
    });
    // Simulating: from .a import func_a (resolved)
    let imp_ba = graph.add_symbol({
        let mut s = make_import("func_a", "mypackage/b.py", 1);
        s.annotations.push("from:a".to_string());
        s.resolution = ResolutionStatus::Resolved;
        s
    });

    // Cross-file edges exist (post-resolution)
    add_ref(
        &mut graph,
        imp_ab,
        func_b,
        ReferenceKind::Import,
        "mypackage/a.py",
    );
    add_ref(
        &mut graph,
        imp_ba,
        func_a,
        ReferenceKind::Import,
        "mypackage/b.py",
    );

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(
        results.len(),
        1,
        "Package-relative imports that are resolved must produce a cycle finding"
    );
}

// ===========================================================================
// Section 5: orphaned_impl / data_dead_end Dedup Core (T16–T23)
// ===========================================================================

/// T16: THE core dedup test. Method with 0 callers appears ONLY in orphaned_impl.
#[test]
fn test_c20_t16_dedup_method_only_in_orphaned_impl() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "process_data",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        10,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        !dead_end_results
            .iter()
            .any(|d| d.entity.contains("process_data")),
        "After dedup, Method 'process_data' must NOT appear in data_dead_end"
    );
    assert!(
        orphaned_results
            .iter()
            .any(|d| d.entity.contains("process_data")),
        "Method 'process_data' with 0 callers must appear in orphaned_impl"
    );
}

/// T17: Private method with zero callers — only orphaned_impl, High confidence.
#[test]
fn test_c20_t17_dedup_private_method_only_orphaned_impl_high_confidence() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "_internal_process",
        SymbolKind::Method,
        Visibility::Private,
        "handler.py",
        15,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        !dead_end_results
            .iter()
            .any(|d| d.entity.contains("_internal_process")),
        "Private Method excluded from data_dead_end"
    );
    assert_eq!(orphaned_results.len(), 1);
    assert_eq!(
        orphaned_results[0].confidence,
        Confidence::High,
        "Private method with 0 callers must have High confidence in orphaned_impl"
    );
}

/// T18: Function with zero callers still appears in data_dead_end (not orphaned_impl).
#[test]
fn test_c20_t18_dedup_function_still_in_data_dead_end() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "unused_helper",
        SymbolKind::Function,
        Visibility::Private,
        "utils.py",
        5,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dead_end_results
            .iter()
            .any(|d| d.entity.contains("unused_helper")),
        "Function with 0 callers must still appear in data_dead_end"
    );
    assert!(
        !orphaned_results
            .iter()
            .any(|d| d.entity.contains("unused_helper")),
        "Function must NOT appear in orphaned_impl (Methods only)"
    );
}

/// T19: Variable with zero readers still appears in data_dead_end.
#[test]
fn test_c20_t19_dedup_variable_still_in_data_dead_end() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "unused_var",
        SymbolKind::Variable,
        Visibility::Private,
        "config.py",
        3,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dead_end_results
            .iter()
            .any(|d| d.entity.contains("unused_var")),
        "Variable must appear in data_dead_end"
    );
    assert!(
        orphaned_results.is_empty(),
        "Variable must NOT appear in orphaned_impl"
    );
}

/// T20: Constant with zero readers still appears in data_dead_end.
#[test]
fn test_c20_t20_dedup_constant_still_in_data_dead_end() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "MAX_RETRIES",
        SymbolKind::Constant,
        Visibility::Public,
        "constants.py",
        1,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    assert!(
        dead_end_results
            .iter()
            .any(|d| d.entity.contains("MAX_RETRIES")),
        "Constant must appear in data_dead_end"
    );
}

/// T21: Method WITH callers appears in neither pattern.
#[test]
fn test_c20_t21_dedup_called_method_in_neither_pattern() {
    let mut graph = Graph::new();

    let method = graph.add_symbol(make_symbol(
        "handle_request",
        SymbolKind::Method,
        Visibility::Public,
        "handler.py",
        10,
    ));
    let caller = graph.add_symbol(make_symbol(
        "dispatch",
        SymbolKind::Function,
        Visibility::Public,
        "router.py",
        5,
    ));
    add_ref(&mut graph, caller, method, ReferenceKind::Call, "router.py");

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        !dead_end_results
            .iter()
            .any(|d| d.entity.contains("handle_request")),
        "Called method must not be in data_dead_end"
    );
    assert!(
        !orphaned_results
            .iter()
            .any(|d| d.entity.contains("handle_request")),
        "Called method must not be in orphaned_impl"
    );
}

/// T22: run_all_patterns produces no duplicate entities for uncalled Methods.
#[test]
fn test_c20_t22_dedup_no_duplicate_entities_in_full_registry() {
    let graph = build_orphaned_method_graph(); // process_user: uncalled Method

    let all_diags = run_all_patterns(&graph, Path::new(""));

    let dead_end_entities: Vec<&str> = all_diags
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::DataDeadEnd)
        .map(|d| d.entity.as_str())
        .collect();
    let orphaned_entities: Vec<&str> = all_diags
        .iter()
        .filter(|d| d.pattern == DiagnosticPattern::OrphanedImplementation)
        .map(|d| d.entity.as_str())
        .collect();

    for entity in &orphaned_entities {
        assert!(
            !dead_end_entities.contains(entity),
            "Entity '{}' appears in BOTH orphaned_impl and data_dead_end — dedup failed",
            entity
        );
    }
}

/// T23: Existing kind exclusions (Module, Class, Struct) still work.
#[test]
fn test_c20_t23_dedup_existing_kind_exclusions_preserved() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "MyModule",
        SymbolKind::Module,
        Visibility::Public,
        "mod.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "MyClass",
        SymbolKind::Class,
        Visibility::Public,
        "cls.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "MyStruct",
        SymbolKind::Struct,
        Visibility::Public,
        "st.rs",
        1,
    ));
    graph.add_symbol(make_symbol(
        "my_method",
        SymbolKind::Method,
        Visibility::Public,
        "svc.py",
        1,
    ));

    let results = data_dead_end::detect(&graph, Path::new(""));

    assert!(
        !results.iter().any(|d| d.entity.contains("MyModule")),
        "Module still excluded"
    );
    assert!(
        !results.iter().any(|d| d.entity.contains("MyClass")),
        "Class still excluded"
    );
    assert!(
        !results.iter().any(|d| d.entity.contains("MyStruct")),
        "Struct still excluded"
    );
    assert!(
        !results.iter().any(|d| d.entity.contains("my_method")),
        "Method now excluded (dedup)"
    );
}

// ===========================================================================
// Section 6: Dedup Adversarial + Cross-Pattern Orthogonality (T24–T29)
// ===========================================================================

/// T24: Trait with zero callers — data_dead_end still detects Traits.
#[test]
fn test_c20_t24_dedup_trait_still_detected_by_data_dead_end() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "Processable",
        SymbolKind::Trait,
        Visibility::Public,
        "traits.rs",
        1,
    ));

    let results = data_dead_end::detect(&graph, Path::new(""));
    assert!(
        results.iter().any(|d| d.entity.contains("Processable")),
        "Trait with 0 callers must still appear in data_dead_end (not excluded by kind filter)"
    );
}

/// T25: Enum, Interface, Macro — data_dead_end still detects non-excluded kinds.
#[test]
fn test_c20_t25_dedup_non_excluded_kinds_still_detected() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "Status",
        SymbolKind::Enum,
        Visibility::Public,
        "types.rs",
        1,
    ));
    graph.add_symbol(make_symbol(
        "Renderable",
        SymbolKind::Interface,
        Visibility::Public,
        "iface.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "debug_print",
        SymbolKind::Macro,
        Visibility::Public,
        "macros.rs",
        1,
    ));

    let results = data_dead_end::detect(&graph, Path::new(""));
    let entities: Vec<&str> = results.iter().map(|d| d.entity.as_str()).collect();

    assert!(
        entities.iter().any(|e| e.contains("Status")),
        "Enum detected"
    );
    assert!(
        entities.iter().any(|e| e.contains("Renderable")),
        "Interface detected"
    );
    assert!(
        entities.iter().any(|e| e.contains("debug_print")),
        "Macro detected"
    );
}

/// T26: Dunder method excluded from orphaned_impl AND data_dead_end.
#[test]
fn test_c20_t26_dedup_dunder_method_still_excluded_from_orphaned_impl() {
    let mut graph = Graph::new();

    graph.add_symbol(make_symbol(
        "__init__",
        SymbolKind::Method,
        Visibility::Public,
        "service.py",
        1,
    ));

    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));
    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));

    assert!(
        orphaned_results.is_empty(),
        "__init__ must be excluded from orphaned_impl (dunder)"
    );
    assert!(
        !dead_end_results
            .iter()
            .any(|d| d.entity.contains("__init__")),
        "__init__ must also be excluded from data_dead_end (Method kind exclusion)"
    );
}

/// T27: Entry-point method excluded from both patterns.
#[test]
fn test_c20_t27_dedup_entry_point_method_excluded_from_both() {
    let mut graph = Graph::new();

    graph.add_symbol(make_entry_point(
        "main_handler",
        SymbolKind::Method,
        Visibility::Public,
        "app.py",
        1,
    ));

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        dead_end_results.is_empty(),
        "Entry point Method excluded from data_dead_end"
    );
    assert!(
        orphaned_results.is_empty(),
        "Entry point Method excluded from orphaned_impl"
    );
}

/// T28: Import-annotated Method excluded from both patterns.
#[test]
fn test_c20_t28_dedup_import_annotated_method_excluded_from_both() {
    let mut graph = Graph::new();

    let mut sym = make_symbol(
        "imported_method",
        SymbolKind::Method,
        Visibility::Public,
        "mod.py",
        1,
    );
    sym.annotations.push("import".to_string());
    graph.add_symbol(sym);

    let dead_end_results = data_dead_end::detect(&graph, Path::new(""));
    let orphaned_results = orphaned_implementation::detect(&graph, Path::new(""));

    assert!(
        !dead_end_results
            .iter()
            .any(|d| d.entity.contains("imported_method")),
        "Import-annotated Method excluded from data_dead_end"
    );
    assert!(
        !orphaned_results
            .iter()
            .any(|d| d.entity.contains("imported_method")),
        "Import-annotated Method excluded from orphaned_impl"
    );
}

/// T29: Cross-pattern orthogonality — no pattern pair produces identical
/// findings on the same entity (same pattern can't fire twice on same entity).
#[test]
fn test_c20_t29_cross_pattern_orthogonality_check() {
    let graph = build_all_fixtures_graph();
    let all_diags = run_all_patterns(&graph, Path::new(""));

    // Group findings by entity
    let mut entity_patterns: HashMap<String, Vec<DiagnosticPattern>> = HashMap::new();
    for d in &all_diags {
        entity_patterns
            .entry(d.entity.clone())
            .or_default()
            .push(d.pattern);
    }

    // Check for duplicate pattern hits on same entity
    for (entity, patterns) in &entity_patterns {
        let unique: HashSet<_> = patterns.iter().collect();
        assert_eq!(
            patterns.len(),
            unique.len(),
            "Entity '{}' has duplicate findings from the same pattern: {:?}",
            entity,
            patterns
        );
    }
}

// ===========================================================================
// Section 7: Dogfood Baseline Impact (T30–T32)
// ===========================================================================

/// Helper: run dogfood analysis on own src/ directory.
fn run_dogfood_c20() -> Vec<crate::manifest::types::DiagnosticEntry> {
    let src_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    assert!(
        src_path.exists(),
        "Source directory not found at {:?}",
        src_path
    );
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let result = crate::analyze(&src_path, &config, &["rust".to_string()])
        .expect("Dogfood analysis must not fail");
    result.manifest.diagnostics
}

/// Helper: count diagnostics matching a pattern name.
fn count_pattern_c20(results: &[crate::manifest::types::DiagnosticEntry], pattern: &str) -> usize {
    results.iter().filter(|d| d.pattern == pattern).count()
}

/// T30: Total finding count decreases after Method exclusion from data_dead_end.
#[test]
fn test_c20_t30_dogfood_total_finding_count_drops_after_dedup() {
    let results = run_dogfood_c20();
    let total = results.len();
    let dead_end = count_pattern_c20(&results, "data_dead_end");
    let orphaned = count_pattern_c20(&results, "orphaned_impl");

    eprintln!(
        "T30: total={}, data_dead_end={}, orphaned_impl={}",
        total, dead_end, orphaned
    );

    // C19 baseline was 588 total, ~311 data_dead_end, ~53 orphaned_impl
    // After dedup, total should drop by roughly the orphaned_impl count
    // Allow wide range for code growth from this cycle's new test files
    assert!(
        total < 588,
        "T30: total={}, expected less than C19 baseline 588 after Method dedup",
        total
    );
}

/// T31: data_dead_end finding count strictly less than before dedup.
#[test]
fn test_c20_t31_data_dead_end_count_drops_after_dedup() {
    let results = run_dogfood_c20();
    let dead_end = count_pattern_c20(&results, "data_dead_end");
    let orphaned = count_pattern_c20(&results, "orphaned_impl");

    eprintln!(
        "T31: data_dead_end={}, orphaned_impl={}",
        dead_end, orphaned
    );

    // data_dead_end should drop because Methods are no longer counted
    // C19 data_dead_end was ~311. After excluding Methods, it should be ~258 or lower.
    // Allow range for code growth from new test files this cycle.
    assert!(
        dead_end < 311,
        "T31: data_dead_end={}, must be less than C19 baseline 311 after Method exclusion",
        dead_end
    );
}

/// T32: Pattern distribution sanity check — no entity appears in both patterns.
#[test]
fn test_c20_t32_pattern_distribution_no_entity_in_both() {
    let results = run_dogfood_c20();

    let dead_end_entities: HashSet<String> = results
        .iter()
        .filter(|d| d.pattern == "data_dead_end")
        .map(|d| d.entity.clone())
        .collect();
    let orphaned_entities: HashSet<String> = results
        .iter()
        .filter(|d| d.pattern == "orphaned_impl")
        .map(|d| d.entity.clone())
        .collect();

    let overlap: HashSet<_> = dead_end_entities.intersection(&orphaned_entities).collect();
    assert!(
        overlap.is_empty(),
        "T32: {} entities appear in BOTH data_dead_end and orphaned_impl: {:?}",
        overlap.len(),
        overlap
    );
}

// ===========================================================================
// Section 8: Regression Tests (T33–T38)
// ===========================================================================

/// T33: circular_dependency confidence is always High.
#[test]
fn test_c20_t33_circular_dep_confidence_always_high() {
    let mut graph = Graph::new();
    let fa = graph.add_symbol(make_symbol(
        "fa",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let fb = graph.add_symbol(make_symbol(
        "fb",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));
    let i_ab = graph.add_symbol({
        let mut s = make_import("fb", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let i_ba = graph.add_symbol({
        let mut s = make_import("fa", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    add_ref(&mut graph, i_ab, fb, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, i_ba, fa, ReferenceKind::Import, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    for r in &results {
        assert_eq!(
            r.confidence,
            Confidence::High,
            "Circular dependencies are structurally verifiable — always High confidence"
        );
    }
}

/// T34: orphaned_impl confidence calibration: Public=Moderate, Private=High.
#[test]
fn test_c20_t34_orphaned_impl_confidence_calibration() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "public_method",
        SymbolKind::Method,
        Visibility::Public,
        "svc.py",
        1,
    ));
    graph.add_symbol(make_symbol(
        "private_method",
        SymbolKind::Method,
        Visibility::Private,
        "svc.py",
        10,
    ));

    let results = orphaned_implementation::detect(&graph, Path::new(""));
    let public_finding = results
        .iter()
        .find(|d| d.entity.contains("public_method"))
        .expect("public_method should be found");
    let private_finding = results
        .iter()
        .find(|d| d.entity.contains("private_method"))
        .expect("private_method should be found");

    assert_eq!(
        public_finding.confidence,
        Confidence::Moderate,
        "Public methods may have dynamic dispatch — Moderate confidence"
    );
    assert_eq!(
        private_finding.confidence,
        Confidence::High,
        "Private methods with 0 callers are high-confidence orphans"
    );
}

/// T35: Underscore-prefixed public method gets High confidence in orphaned_impl.
#[test]
fn test_c20_t35_orphaned_impl_underscore_public_method_high_confidence() {
    let mut graph = Graph::new();
    graph.add_symbol(make_symbol(
        "_internal_handler",
        SymbolKind::Method,
        Visibility::Public,
        "svc.py",
        1,
    ));

    let results = orphaned_implementation::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].confidence,
        Confidence::High,
        "Underscore-prefixed public method is likely internal — High confidence"
    );
}

/// T36: Field test regression — orphaned_impl count unchanged by dedup.
#[test]
fn test_c20_t36_orphaned_impl_count_unchanged_by_dedup() {
    let results = run_dogfood_c20();
    let orphaned = count_pattern_c20(&results, "orphaned_impl");

    eprintln!("T36: orphaned_impl={}", orphaned);

    // orphaned_impl count should be approximately the same as before dedup
    // C18 baseline was 53. Allow range for code growth.
    assert!(
        orphaned >= 40 && orphaned <= 100,
        "T36: orphaned_impl={}, expected 40-100 (C18 baseline was 53, unchanged by dedup)",
        orphaned
    );
}

/// T37: Cycle detection message format includes module count and path.
#[test]
fn test_c20_t37_circular_dep_message_format() {
    let mut graph = Graph::new();
    let fa = graph.add_symbol(make_symbol(
        "fa",
        SymbolKind::Function,
        Visibility::Public,
        "a.py",
        5,
    ));
    let fb = graph.add_symbol(make_symbol(
        "fb",
        SymbolKind::Function,
        Visibility::Public,
        "b.py",
        5,
    ));
    let i_ab = graph.add_symbol({
        let mut s = make_import("fb", "a.py", 1);
        s.annotations.push("from:b".to_string());
        s
    });
    let i_ba = graph.add_symbol({
        let mut s = make_import("fa", "b.py", 1);
        s.annotations.push("from:a".to_string());
        s
    });
    add_ref(&mut graph, i_ab, fb, ReferenceKind::Import, "a.py");
    add_ref(&mut graph, i_ba, fa, ReferenceKind::Import, "b.py");

    let results = circular_dependency::detect(&graph, Path::new(""));
    assert_eq!(results.len(), 1);
    assert!(
        results[0].message.contains("2 modules"),
        "Message must include module count, got: {}",
        results[0].message
    );
    assert!(
        results[0].message.contains("->"),
        "Message must show cycle path with arrows, got: {}",
        results[0].message
    );
}

/// T38: Empty graph produces no panics across all three patterns.
#[test]
fn test_c20_t38_empty_graph_no_panics_either_pattern() {
    let graph = Graph::new();
    let dead_end = data_dead_end::detect(&graph, Path::new(""));
    let orphaned = orphaned_implementation::detect(&graph, Path::new(""));
    let circular = circular_dependency::detect(&graph, Path::new(""));

    assert!(dead_end.is_empty());
    assert!(orphaned.is_empty());
    assert!(circular.is_empty());
}
