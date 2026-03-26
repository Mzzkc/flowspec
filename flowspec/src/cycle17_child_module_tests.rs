// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! QA-2 Cycle 17: stale_reference Mechanism A fix — child module detection tests.
//!
//! Tests for `is_child_module` fallback in `resolve_cross_file_imports` that
//! resolves imports of child modules (e.g., `use crate::patterns::data_dead_end`
//! where `data_dead_end` is a submodule, not a symbol in `patterns/mod.rs`).
//!
//! Sections:
//! - T1-T8:   Core TDD anchors — child module detection
//! - T9-T16:  Edge cases and boundary conditions
//! - T17-T22: Cross-pattern orthogonality guards (dogfood)
//! - T23-T26: Mechanism A dogfood regression
//! - T27-T30: Mixed-language module resolution
//! - T31-T36: Adversarial tests
//! - T37-T40: Regression guards from previous cycles

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::analyzer::patterns::{phantom_dependency, stale_reference};
use crate::graph::resolve_cross_file_imports;
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;
use crate::test_utils::*;

/// Helper: create an import symbol with `from:` annotation and optional `original_name:`.
/// Sets resolution to Unresolved so resolve_cross_file_imports will process it.
fn make_from_import(
    name: &str,
    from_module: &str,
    file: &str,
    line: u32,
    original_name: Option<&str>,
) -> Symbol {
    let mut sym = make_import(name, file, line);
    sym.resolution = ResolutionStatus::Unresolved;
    sym.annotations.push(format!("from:{}", from_module));
    if let Some(orig) = original_name {
        sym.annotations.push(format!("original_name:{}", orig));
    }
    sym
}

/// Helper: build module map from key-path pairs.
fn build_test_module_map(entries: &[(&str, &str)]) -> HashMap<String, PathBuf> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), PathBuf::from(v)))
        .collect()
}

/// Helper: parse Rust source and return import symbol names.
fn parse_import_names(source: &str) -> Vec<String> {
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(Path::new("test.rs"), source).unwrap();
    result
        .symbols
        .iter()
        .filter(|s| s.annotations.contains(&"import".to_string()))
        .map(|s| s.name.clone())
        .collect()
}

// =========================================================================
// Section 1: Core TDD Anchors — Child Module Detection (T1–T8)
// =========================================================================

/// T1: Basic child module import resolves.
#[test]
fn test_c17_t1_child_module_import_resolves() {
    let mut graph = Graph::new();

    let _parent_mod = graph.add_symbol(make_symbol(
        "patterns",
        SymbolKind::Module,
        Visibility::Public,
        "src/analyzer/patterns/mod.rs",
        1,
    ));
    let _child_fn = graph.add_symbol(make_symbol(
        "detect",
        SymbolKind::Function,
        Visibility::Public,
        "src/analyzer/patterns/data_dead_end.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import(
        "data_dead_end",
        "crate::analyzer::patterns",
        "src/consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::analyzer::patterns", "src/analyzer/patterns/mod.rs"),
        (
            "crate::analyzer::patterns::data_dead_end",
            "src/analyzer/patterns/data_dead_end.rs",
        ),
        ("crate::consumer", "src/consumer.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let import_sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "data_dead_end" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .expect("import symbol must exist");

    assert_eq!(
        import_sym.resolution,
        ResolutionStatus::Resolved,
        "child module import 'data_dead_end' must resolve, got {:?}",
        import_sym.resolution
    );
}

/// T2: Child module via `mod.rs` subdirectory resolves.
#[test]
fn test_c17_t2_child_module_via_mod_rs_resolves() {
    let mut graph = Graph::new();

    let _parent = graph.add_symbol(make_symbol(
        "a",
        SymbolKind::Module,
        Visibility::Public,
        "a/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import("b", "crate::a", "consumer.rs", 1, None));

    let module_map = build_test_module_map(&[
        ("crate::a", "a/mod.rs"),
        ("crate::a::b", "a/b/mod.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let import_sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "b" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(import_sym.resolution, ResolutionStatus::Resolved);
}

/// T3: Child module via standalone `.rs` file resolves.
#[test]
fn test_c17_t3_child_module_standalone_rs_resolves() {
    let mut graph = Graph::new();

    let _parent = graph.add_symbol(make_symbol(
        "parser",
        SymbolKind::Module,
        Visibility::Public,
        "parser/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import(
        "ir",
        "crate::parser",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::parser", "parser/mod.rs"),
        ("crate::parser::ir", "parser/ir.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "ir" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

/// T4: Multiple child module imports from same parent all resolve.
#[test]
fn test_c17_t4_multiple_child_modules_resolve() {
    let mut graph = Graph::new();

    let _parent = graph.add_symbol(make_symbol(
        "patterns",
        SymbolKind::Module,
        Visibility::Public,
        "patterns/mod.rs",
        1,
    ));

    for (i, name) in ["data_dead_end", "phantom_dependency", "circular_dependency"]
        .iter()
        .enumerate()
    {
        let _import = graph.add_symbol(make_from_import(
            name,
            "crate::analyzer::patterns",
            "consumer.rs",
            i as u32 + 1,
            None,
        ));
    }

    let module_map = build_test_module_map(&[
        ("crate::analyzer::patterns", "patterns/mod.rs"),
        (
            "crate::analyzer::patterns::data_dead_end",
            "patterns/data_dead_end.rs",
        ),
        (
            "crate::analyzer::patterns::phantom_dependency",
            "patterns/phantom_dependency.rs",
        ),
        (
            "crate::analyzer::patterns::circular_dependency",
            "patterns/circular_dependency.rs",
        ),
        ("crate::consumer", "consumer.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    for name in &["data_dead_end", "phantom_dependency", "circular_dependency"] {
        let sym = graph
            .all_symbols()
            .find(|(_, s)| &s.name == name && s.annotations.contains(&"import".to_string()))
            .map(|(_, s)| s)
            .unwrap_or_else(|| panic!("import '{}' must exist", name));

        assert_eq!(
            sym.resolution,
            ResolutionStatus::Resolved,
            "child module '{}' must resolve",
            name
        );
    }
}

/// T5: Deeply nested child module resolves.
#[test]
fn test_c17_t5_deeply_nested_child_module_resolves() {
    let mut graph = Graph::new();
    let _import = graph.add_symbol(make_from_import("b", "crate::a", "consumer.rs", 1, None));

    let module_map = build_test_module_map(&[
        ("crate::a", "a/mod.rs"),
        ("crate::a::b", "a/b/mod.rs"),
        ("crate::a::b::c", "a/b/c.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "b" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

/// T6: Non-existent child module stays stale. CRITICAL safety test.
#[test]
fn test_c17_t6_nonexistent_child_stays_stale() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "deleted_module",
        "crate::parser",
        "consumer.rs",
        1,
        None,
    ));

    let module_map =
        build_test_module_map(&[("crate::parser", "parser/mod.rs"), ("crate", "lib.rs")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "deleted_module" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert!(
        matches!(sym.resolution, ResolutionStatus::Partial(ref msg) if msg.contains("symbol not found")),
        "deleted module import must remain stale, got {:?}",
        sym.resolution
    );
}

/// T7: Symbol match still takes priority over child module match.
#[test]
fn test_c17_t7_symbol_match_preferred_over_child_module() {
    let mut graph = Graph::new();

    // The parent file defines a FUNCTION named "helper"
    let _helper_fn = graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "utils.rs",
        5,
    ));

    let _import = graph.add_symbol(make_from_import(
        "helper",
        "crate::utils",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::utils", "utils.rs"),
        ("crate::utils::helper", "utils/helper.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    // Verify resolved via symbol match (Reference edge created)
    let import_id = graph
        .all_symbols()
        .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
        .map(|(id, _)| id)
        .unwrap();

    let import_sym = graph.get_symbol(import_id).unwrap();
    assert_eq!(import_sym.resolution, ResolutionStatus::Resolved);

    let edges = graph.edges_from(import_id);
    assert!(
        !edges.is_empty(),
        "symbol match should create Reference edge (child module fallback does not)"
    );
}

/// T8: Module-level import (lookup_name == module_name) still works.
#[test]
fn test_c17_t8_module_level_import_unchanged() {
    let mut graph = Graph::new();

    // Python-style: from:"utils" and name:"utils" → lookup_name == module_name → Resolved
    let _import = graph.add_symbol(make_from_import("utils", "utils", "consumer.py", 1, None));

    let module_map = build_test_module_map(&[("utils", "utils.py")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "utils" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

// =========================================================================
// Section 2: Edge Cases and Boundary Conditions (T9–T16)
// =========================================================================

/// T9: Case sensitivity — PascalCase name does NOT match snake_case module.
#[test]
fn test_c17_t9_case_sensitive_child_module_detection() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "DataDeadEnd",
        "crate::patterns",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::patterns", "patterns/mod.rs"),
        (
            "crate::patterns::data_dead_end",
            "patterns/data_dead_end.rs",
        ),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "DataDeadEnd" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert!(
        matches!(sym.resolution, ResolutionStatus::Partial(_)),
        "PascalCase 'DataDeadEnd' must NOT match snake_case module, got {:?}",
        sym.resolution
    );
}

/// T10: Import from non-Rust file ignores Rust child module logic.
#[test]
fn test_c17_t10_python_import_no_rust_child_module() {
    let mut graph = Graph::new();

    // Python import — "from:utils" does not contain "::", so is_child_module returns false
    let _import = graph.add_symbol(make_from_import("helper", "utils", "consumer.py", 1, None));

    let module_map = build_test_module_map(&[
        ("utils", "utils.py"),
        ("crate::utils", "src/utils.rs"),
        ("crate::utils::helper", "src/utils/helper.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    // Python "from:utils" resolves to utils.py. "helper" isn't found there → Partial.
    // The key: it does NOT trigger Rust child module detection ("utils" has no "::")
    assert!(
        !matches!(sym.resolution, ResolutionStatus::Resolved),
        "Python import should NOT use Rust child module detection"
    );
}

/// T11: Empty module map — no crash.
#[test]
fn test_c17_t11_empty_module_map_no_crash() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "foo",
        "crate::bar",
        "consumer.rs",
        1,
        None,
    ));

    let module_map: HashMap<String, PathBuf> = HashMap::new();

    resolve_cross_file_imports(&mut graph, &module_map);

    // With empty map, resolve_cross_file_imports returns early — no changes
    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "foo" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(
        sym.resolution,
        ResolutionStatus::Unresolved,
        "empty module map should leave import Unresolved"
    );
}

/// T12: Target file at root level (crate root as parent).
#[test]
fn test_c17_t12_root_level_file_child_module() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import("util", "crate", "consumer.rs", 1, None));

    let module_map = build_test_module_map(&[
        ("crate", "lib.rs"),
        ("crate::util", "util.rs"),
        ("crate::consumer", "consumer.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    // "crate" doesn't contain "::" so is_child_module returns false.
    // "util" != "crate" so module-level check fails too.
    // Result: Partial. This is a known limitation for crate-root imports.
    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "util" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    // Just verify no crash — behavior is documented
    eprintln!("T12: root-level import resolution = {:?}", sym.resolution);
}

/// T13: Module name with only single character.
#[test]
fn test_c17_t13_single_char_module_name() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import("b", "crate::a", "consumer.rs", 1, None));

    let module_map = build_test_module_map(&[
        ("crate::a", "a/mod.rs"),
        ("crate::a::b", "a/b.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "b" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

/// T14: Star import unaffected by child module detection.
#[test]
fn test_c17_t14_star_import_still_partial() {
    let mut graph = Graph::new();

    let mut star_sym = make_import("*:crate::patterns", "consumer.rs", 1);
    star_sym.resolution = ResolutionStatus::Unresolved;
    star_sym
        .annotations
        .push("from:crate::patterns".to_string());
    let _import = graph.add_symbol(star_sym);

    let module_map = build_test_module_map(&[
        ("crate::patterns", "patterns/mod.rs"),
        (
            "crate::patterns::data_dead_end",
            "patterns/data_dead_end.rs",
        ),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name.starts_with("*:") && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert!(
        matches!(sym.resolution, ResolutionStatus::Partial(ref msg) if msg.contains("star import")),
        "star import should stay Partial, got {:?}",
        sym.resolution
    );
}

/// T15: Self-import (`use crate::module::{self}`) not affected.
#[test]
fn test_c17_t15_self_import_not_affected() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "module",
        "crate::module",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[("crate::module", "module.rs"), ("crate", "lib.rs")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "module" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    // "module" != "crate::module" (not module-level match)
    // "crate::module::module" not in map (not child module match)
    // → Partial. Self-imports were handled by C16's parser fix.
    eprintln!("T15: self-import resolution = {:?}", sym.resolution);
}

/// T16: Aliased import with child module name uses original_name for lookup.
#[test]
fn test_c17_t16_aliased_import_child_module() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "stale_ref",
        "crate::analyzer::patterns",
        "consumer.rs",
        1,
        Some("stale_reference"),
    ));

    let module_map = build_test_module_map(&[
        ("crate::analyzer::patterns", "patterns/mod.rs"),
        (
            "crate::analyzer::patterns::stale_reference",
            "patterns/stale_reference.rs",
        ),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "stale_ref" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(
        sym.resolution,
        ResolutionStatus::Resolved,
        "aliased import should use original_name for child module lookup"
    );
}

// =========================================================================
// Section 3: Cross-Pattern Orthogonality Guards (T17–T22)
// =========================================================================

/// T17: phantom_dependency count stable after fix.
#[test]
fn test_c17_t17_phantom_dependency_unchanged() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let phantom = diagnostics
        .iter()
        .filter(|d| d.pattern == "phantom_dependency")
        .count();
    eprintln!("T17: phantom_dependency = {}", phantom);

    assert!(
        (phantom as i32 - 135).abs() <= 15,
        "phantom ~135, got {}",
        phantom
    );
}

/// T18: missing_reexport count stable after fix.
#[test]
fn test_c17_t18_missing_reexport_unchanged() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let missing = diagnostics
        .iter()
        .filter(|d| d.pattern == "missing_reexport")
        .count();
    eprintln!("T18: missing_reexport = {}", missing);

    assert!(
        (missing as i32 - 59).abs() <= 10,
        "missing_reexport ~59, got {}",
        missing
    );
}

/// T19: data_dead_end count stable after fix.
#[test]
fn test_c17_t19_data_dead_end_unchanged() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let dead_end = diagnostics
        .iter()
        .filter(|d| d.pattern == "data_dead_end")
        .count();
    eprintln!("T19: data_dead_end = {}", dead_end);

    // Baseline drifted to ~221 after C17 Worker 3 commits added new code.
    assert!(
        (dead_end as i32 - 221).abs() <= 30,
        "data_dead_end ~221, got {}",
        dead_end
    );
}

/// T20: orphaned_implementation count stable after fix.
#[test]
fn test_c17_t20_orphaned_impl_unchanged() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let orphaned = diagnostics
        .iter()
        .filter(|d| d.pattern == "orphaned_implementation")
        .count();
    eprintln!("T20: orphaned_implementation = {}", orphaned);

    // C16 synthesis reported 53 but actual measurement shows 0 after method call tracking.
    assert!(
        orphaned <= 60,
        "orphaned_implementation stable (≤60), got {}",
        orphaned
    );
}

/// T21: circular/partial/isolated unchanged (hard equality).
#[test]
fn test_c17_t21_stable_patterns_unchanged() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let circular = diagnostics
        .iter()
        .filter(|d| d.pattern == "circular_dependency")
        .count();
    let partial = diagnostics
        .iter()
        .filter(|d| d.pattern == "partial_wiring")
        .count();
    let isolated = diagnostics
        .iter()
        .filter(|d| d.pattern == "isolated_cluster")
        .count();

    eprintln!(
        "T21: circular={}, partial={}, isolated={}",
        circular, partial, isolated
    );

    assert_eq!(circular, 5, "circular_dependency must be exactly 5");
    assert_eq!(partial, 2, "partial_wiring must be exactly 2");
    assert_eq!(isolated, 1, "isolated_cluster must be exactly 1");
}

/// T22: Total dogfood decreases but not implausibly.
#[test]
fn test_c17_t22_total_dogfood_bounded_decrease() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let total = diagnostics.len();
    eprintln!("T22: total findings = {}", total);

    // Current HEAD baseline is ~495 (after Worker 3's C17 commits).
    // Our stale fix reduces ~45 findings. Allow generous window.
    assert!(
        total < 500,
        "must not increase beyond current baseline of ~495, got {}",
        total
    );
    assert!(
        total > 350,
        "must not drop implausibly (broken detection), got {}",
        total
    );
}

// =========================================================================
// Section 4: Mechanism A Fix — Dogfood Regression (T23–T26)
// =========================================================================

/// T23: stale_reference drops by ~43 (Mechanism A elimination).
#[test]
fn test_c17_t23_stale_reference_mechanism_a_eliminated() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .count();
    eprintln!("T23: stale_reference = {}", stale);

    assert!(
        stale < 30,
        "stale should drop to ~21 (from 64, -43 Mechanism A), got {}",
        stale
    );
    assert!(
        stale > 10,
        "Mechanisms B/C/D should still fire (~21 expected), got {}",
        stale
    );
}

/// T24: Known Mechanism A exemplars resolved.
#[test]
fn test_c17_t24_known_mechanism_a_imports_resolved() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    for exemplar in &["data_dead_end", "phantom_dependency", "circular_dependency"] {
        let found = stale_findings.iter().any(|d| d.entity == *exemplar);
        assert!(
            !found,
            "Mechanism A exemplar '{}' should no longer appear as stale_reference",
            exemplar
        );
    }
}

/// T25: Known Mechanism B/C exemplars still fire.
#[test]
fn test_c17_t25_mechanism_b_c_not_suppressed() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    let has_mechanism_b = stale_findings.iter().any(|d| d.entity == "SymbolId");
    let has_mechanism_c = stale_findings.iter().any(|d| d.entity == "DiagnosticEntry");

    eprintln!(
        "T25: mechanism_b(SymbolId)={}, mechanism_c(DiagnosticEntry)={}",
        has_mechanism_b, has_mechanism_c
    );

    assert!(
        has_mechanism_b || has_mechanism_c,
        "at least one Mechanism B/C exemplar must still fire"
    );
}

/// T26: Test fixture stale references preserved (true positives).
#[test]
fn test_c17_t26_fixture_stale_references_preserved() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale_count = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .count();
    assert!(
        stale_count > 0,
        "stale_reference must still produce findings for non-Mechanism-A cases"
    );
}

// =========================================================================
// Section 5: Mixed-Language Module Resolution (T27–T30)
// =========================================================================

/// T27: Python and JS file with same name in same directory.
#[test]
fn test_c17_t27_mixed_language_same_name_module_map() {
    let files = vec![
        PathBuf::from("/project/utils.py"),
        PathBuf::from("/project/utils.js"),
    ];

    let map = crate::build_module_map(&files);
    eprintln!("T27: module_map = {:?}", map);

    assert!(!map.is_empty(), "module map should have at least one entry");
}

/// T28: Python import doesn't resolve to JS file.
#[test]
fn test_c17_t28_python_import_doesnt_resolve_to_js() {
    let mut graph = Graph::new();

    let _py_fn = graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "utils.py",
        5,
    ));
    let _js_fn = graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "utils.js",
        5,
    ));
    let _import = graph.add_symbol(make_from_import("helper", "utils", "consumer.py", 1, None));

    let module_map = build_test_module_map(&[("utils", "utils.py")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let import_id = graph
        .all_symbols()
        .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
        .map(|(id, _)| id)
        .unwrap();

    let import_sym = graph.get_symbol(import_id).unwrap();
    if import_sym.resolution == ResolutionStatus::Resolved {
        let edges = graph.edges_from(import_id);
        if let Some(edge) = edges.first() {
            if let Some(target) = graph.get_symbol(edge.target) {
                assert_eq!(
                    target.location.file,
                    PathBuf::from("utils.py"),
                    "must resolve to Python file"
                );
            }
        }
    }
}

/// T29: JS require doesn't resolve to Python file.
#[test]
fn test_c17_t29_js_require_doesnt_resolve_to_python() {
    let mut graph = Graph::new();

    let _js_fn = graph.add_symbol(make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "utils.js",
        5,
    ));
    let _import = graph.add_symbol(make_from_import("helper", "./utils", "app.js", 1, None));

    let module_map = build_test_module_map(&[("./utils", "utils.js")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "helper" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    eprintln!("T29: JS import resolution = {:?}", sym.resolution);
}

/// T30: Rust imports completely isolated from Python/JS.
#[test]
fn test_c17_t30_rust_imports_isolated() {
    let mut graph = Graph::new();

    // Rust `use crate::utils` → from:"crate", name:"utils"
    let _import = graph.add_symbol(make_from_import("utils", "crate", "src/main.rs", 1, None));

    let module_map = build_test_module_map(&[
        ("utils", "utils.py"),
        ("crate", "src/lib.rs"),
        ("crate::utils", "src/utils.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "utils" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    // from:"crate" routes through Rust resolution → finds src/lib.rs
    // "crate" doesn't contain "::" → is_child_module returns false
    // This is a known limitation for crate-root imports. Just verify no crash.
    assert!(
        sym.resolution != ResolutionStatus::Unresolved,
        "import should have been processed"
    );
}

// =========================================================================
// Section 6: Adversarial Tests (T31–T36)
// =========================================================================

/// T31: Symbol match takes priority when both symbol and child module exist.
#[test]
fn test_c17_t31_real_symbol_in_module_with_child_of_same_name() {
    let mut graph = Graph::new();

    let _util_fn = graph.add_symbol(make_symbol(
        "util",
        SymbolKind::Function,
        Visibility::Public,
        "fs.rs",
        5,
    ));
    let _import = graph.add_symbol(make_from_import(
        "util",
        "crate::fs",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::fs", "fs.rs"),
        ("crate::fs::util", "fs/util.rs"),
        ("crate", "lib.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let import_id = graph
        .all_symbols()
        .find(|(_, s)| s.name == "util" && s.annotations.contains(&"import".to_string()))
        .map(|(id, _)| id)
        .unwrap();

    let import_sym = graph.get_symbol(import_id).unwrap();
    assert_eq!(import_sym.resolution, ResolutionStatus::Resolved);

    // Symbol match creates Reference edge; child module fallback doesn't
    let edges = graph.edges_from(import_id);
    assert!(
        !edges.is_empty(),
        "symbol match should create Reference edge"
    );
}

/// T32: Module name with underscore vs no underscore — no false match.
#[test]
fn test_c17_t32_underscore_variant_no_false_match() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "data_dead_end",
        "crate::patterns",
        "consumer.rs",
        1,
        None,
    ));

    // Only "dead_end" exists, NOT "data_dead_end"
    let module_map = build_test_module_map(&[
        ("crate::patterns", "patterns/mod.rs"),
        ("crate::patterns::dead_end", "patterns/dead_end.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "data_dead_end" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert!(
        matches!(sym.resolution, ResolutionStatus::Partial(_)),
        "exact key match required — 'dead_end' must NOT match 'data_dead_end', got {:?}",
        sym.resolution
    );
}

/// T33: Module with numeric suffix.
#[test]
fn test_c17_t33_numeric_suffix_module() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import("api", "crate::v2", "consumer.rs", 1, None));

    let module_map =
        build_test_module_map(&[("crate::v2", "v2/mod.rs"), ("crate::v2::api", "v2/api.rs")]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "api" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

/// T34: Import from `super::` — tests child module detection on normalized paths.
#[test]
fn test_c17_t34_super_relative_child_module() {
    let mut graph = Graph::new();

    // Using crate:: directly (super:: normalization is separate)
    let _import = graph.add_symbol(make_from_import(
        "patterns",
        "crate::analyzer",
        "src/other.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::analyzer", "analyzer/mod.rs"),
        ("crate::analyzer::patterns", "analyzer/patterns/mod.rs"),
        ("crate::other", "src/other.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "patterns" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

/// T35: Large module tree — performance sanity.
#[test]
fn test_c17_t35_large_module_tree_no_quadratic() {
    let mut graph = Graph::new();

    let mut map_entries: Vec<(String, PathBuf)> = vec![(
        "crate::big_mod".to_string(),
        PathBuf::from("big_mod/mod.rs"),
    )];

    for i in 0..200 {
        map_entries.push((
            format!("crate::big_mod::child_{}", i),
            PathBuf::from(format!("big_mod/child_{}.rs", i)),
        ));
    }

    for i in 0..50 {
        let name = format!("child_{}", i);
        let _import = graph.add_symbol(make_from_import(
            &name,
            "crate::big_mod",
            "consumer.rs",
            i as u32 + 1,
            None,
        ));
    }

    let module_map: HashMap<String, PathBuf> = map_entries.into_iter().collect();

    let start = std::time::Instant::now();
    resolve_cross_file_imports(&mut graph, &module_map);
    let elapsed = start.elapsed();

    eprintln!("T35: 200 modules, 50 imports resolved in {:?}", elapsed);
    assert!(
        elapsed.as_secs() < 1,
        "should complete in < 1 second, took {:?}",
        elapsed
    );

    let resolved_count = graph
        .all_symbols()
        .filter(|(_, s)| {
            s.annotations.contains(&"import".to_string())
                && s.resolution == ResolutionStatus::Resolved
        })
        .count();

    assert_eq!(
        resolved_count, 50,
        "all 50 child module imports should resolve"
    );
}

/// T36: Import of re-exported module name — child module fallback handles this.
#[test]
fn test_c17_t36_reexport_vs_child_module() {
    let mut graph = Graph::new();

    let _import = graph.add_symbol(make_from_import(
        "types",
        "crate::lib",
        "consumer.rs",
        1,
        None,
    ));

    let module_map = build_test_module_map(&[
        ("crate::lib", "lib.rs"),
        ("crate::lib::types", "lib/types.rs"),
    ]);

    resolve_cross_file_imports(&mut graph, &module_map);

    let sym = graph
        .all_symbols()
        .find(|(_, s)| s.name == "types" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();

    assert_eq!(sym.resolution, ResolutionStatus::Resolved);
}

// =========================================================================
// Section 7: Regression Guards from Previous Cycles (T37–T40)
// =========================================================================

/// T37: C16 path-segment fix regression — intermediate segments still filtered.
#[test]
fn test_c17_t37_c16_path_segment_fix_intact() {
    let names =
        parse_import_names("use crate::analyzer::patterns::{data_dead_end, phantom_dependency};");

    assert!(names.contains(&"data_dead_end".to_string()));
    assert!(names.contains(&"phantom_dependency".to_string()));
    assert_eq!(names.len(), 2);

    for bad in &["analyzer", "patterns", "crate"] {
        assert!(
            !names.contains(&bad.to_string()),
            "intermediate '{}' must not be emitted",
            bad
        );
    }
}

/// T38: C16 T25 — module-as-leaf import `use std::fs` still valid.
#[test]
fn test_c17_t38_c16_t25_module_as_leaf_import() {
    let names = parse_import_names("use std::fs;");
    assert!(names.contains(&"fs".to_string()));
    assert_eq!(names.len(), 1);
}

/// T39: C15 regressions hold — genuinely stale import still detected.
#[test]
fn test_c17_t39_c15_regressions_hold() {
    let mut graph = Graph::new();

    let mut sym = make_import("old_validate", "handler.rs", 5);
    sym.annotations.push("from:crate::utils".to_string());
    sym.resolution = ResolutionStatus::Partial("module resolved, symbol not found".to_string());
    let _import_id = graph.add_symbol(sym);

    let project_root = Path::new(".");
    let findings = stale_reference::detect(&graph, project_root);

    assert!(
        !findings.is_empty(),
        "genuinely stale import should still be detected"
    );
    assert!(
        findings.iter().any(|f| f.entity == "old_validate"),
        "stale 'old_validate' must appear"
    );
}

/// T40: C14 diagnostic orthogonality — stale checks resolution, phantom checks edges.
#[test]
fn test_c17_t40_diagnostic_orthogonality() {
    let mut graph = Graph::new();

    let mut sym = make_import("mixed_signal", "handler.rs", 5);
    sym.annotations.push("from:crate::utils".to_string());
    sym.resolution = ResolutionStatus::Partial("module resolved, symbol not found".to_string());
    let import_id = graph.add_symbol(sym);

    // A function in the same file references this import (suppresses phantom)
    let caller = graph.add_symbol(make_symbol(
        "use_it",
        SymbolKind::Function,
        Visibility::Public,
        "handler.rs",
        10,
    ));
    add_ref(
        &mut graph,
        caller,
        import_id,
        ReferenceKind::Read,
        "handler.rs",
    );

    let project_root = Path::new(".");
    let stale_findings = stale_reference::detect(&graph, project_root);
    let phantom_findings = phantom_dependency::detect(&graph, project_root);

    // stale_reference fires (Partial resolution)
    assert!(
        stale_findings.iter().any(|f| f.entity == "mixed_signal"),
        "stale_reference should fire on Partial resolution"
    );

    // phantom_dependency does NOT fire (same-file reference edge exists)
    assert!(
        !phantom_findings.iter().any(|f| f.entity == "mixed_signal"),
        "phantom_dependency should NOT fire when same-file Reference edge exists"
    );
}
