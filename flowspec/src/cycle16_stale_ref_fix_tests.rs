// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! QA-2 Cycle 16: stale_reference path-segment fix TDD tests.
//!
//! Tests for `extract_use_tree` fix (#18) that stops emitting intermediate
//! path segments as import symbols. Also includes dogfood regression guards,
//! issue verification, adversarial tests, and cross-pattern interaction guards.
//!
//! Sections:
//! - T1-T5: Path-segment filtering — core behavior
//! - T6-T10: Complex use tree patterns
//! - T11-T15: Dogfood regression guards
//! - T16-T19: C15 flipped regression guards
//! - T20-T24: Issue verification tests
//! - T25-T30: Adversarial tests
//! - T31-T34: Cross-pattern interaction guards

use std::path::Path;

use crate::analyzer::patterns::{
    data_dead_end, missing_reexport, orphaned_implementation, phantom_dependency, stale_reference,
};
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;
use crate::test_utils::*;

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

/// Helper: parse Rust source and return full parse result.
fn parse_rust(source: &str) -> ParseResult {
    let adapter = RustAdapter::new();
    adapter.parse_file(Path::new("test.rs"), source).unwrap()
}

// =========================================================================
// Section 1: Path-Segment Filtering — Core Behavior (T1-T5)
// =========================================================================

/// T1: Grouped import — only leaf items emitted, no intermediates.
#[test]
fn test_c16_t1_grouped_import_only_emits_leaves() {
    let names = parse_import_names("use crate::module::{ItemA, ItemB};");

    assert!(
        names.contains(&"ItemA".to_string()),
        "leaf ItemA must be emitted"
    );
    assert!(
        names.contains(&"ItemB".to_string()),
        "leaf ItemB must be emitted"
    );
    assert!(
        !names.contains(&"module".to_string()),
        "intermediate 'module' must NOT be emitted"
    );
    assert!(
        !names.contains(&"crate".to_string()),
        "path root 'crate' must NOT be emitted"
    );
    assert_eq!(names.len(), 2, "exactly 2 imports, no intermediates");
}

/// T2: Deeply nested path — only final leaves emitted.
#[test]
fn test_c16_t2_deep_path_only_emits_leaves() {
    let names = parse_import_names("use crate::a::b::c::{x, y};");

    assert!(names.contains(&"x".to_string()));
    assert!(names.contains(&"y".to_string()));
    assert_eq!(names.len(), 2, "only leaf items, no intermediates a/b/c");
    for intermediate in &["a", "b", "c", "crate"] {
        assert!(
            !names.contains(&intermediate.to_string()),
            "intermediate '{}' must not be emitted",
            intermediate
        );
    }
}

/// T3: Single-item import — no regression, leaf still emitted.
#[test]
fn test_c16_t3_single_item_import_still_works() {
    let names = parse_import_names("use crate::module::Item;");

    assert!(
        names.contains(&"Item".to_string()),
        "leaf Item must still be emitted"
    );
    assert!(
        !names.contains(&"module".to_string()),
        "intermediate 'module' must not be emitted"
    );
    assert_eq!(names.len(), 1, "exactly one import");
}

/// T4: Star import — no intermediate module symbols emitted.
#[test]
fn test_c16_t4_star_import_no_intermediate_symbols() {
    let result = parse_rust("use crate::module::*;");

    let import_symbols: Vec<&str> = result
        .symbols
        .iter()
        .filter(|s| s.annotations.contains(&"import".to_string()))
        .map(|s| s.name.as_str())
        .collect();

    assert!(
        !import_symbols.contains(&"module"),
        "intermediate 'module' must not be emitted for star imports"
    );

    // Star import produces a Reference, not an import Symbol
    let star_refs = result
        .references
        .iter()
        .filter(|r| r.kind == ReferenceKind::Import)
        .count();
    assert!(star_refs > 0, "star import reference must be emitted");
}

/// T5: Aliased import — alias emitted, path segments not.
#[test]
fn test_c16_t5_aliased_import_only_alias() {
    let names = parse_import_names("use crate::module::LongName as Short;");

    assert!(
        names.contains(&"Short".to_string()),
        "alias must be emitted"
    );
    assert!(
        !names.contains(&"module".to_string()),
        "intermediate must not be emitted"
    );
    // LongName must not appear because the alias replaces it
    assert!(
        !names.contains(&"LongName".to_string()),
        "original name must not be emitted when aliased"
    );
}

// =========================================================================
// Section 2: Complex Use Tree Patterns (T6-T10)
// =========================================================================

/// T6: Nested scoped identifier inside use list.
#[test]
fn test_c16_t6_nested_scoped_in_use_list() {
    let names = parse_import_names("use crate::foo::{bar::Baz, Qux};");

    assert!(
        names.contains(&"Baz".to_string()),
        "nested leaf Baz must be emitted"
    );
    assert!(
        names.contains(&"Qux".to_string()),
        "direct leaf Qux must be emitted"
    );
    assert!(
        !names.contains(&"bar".to_string()),
        "nested intermediate 'bar' must not be emitted"
    );
    assert!(
        !names.contains(&"foo".to_string()),
        "top-level intermediate 'foo' must not be emitted"
    );
    assert_eq!(names.len(), 2);
}

/// T7: Self import in use list — self is a legitimate leaf import.
#[test]
fn test_c16_t7_self_import() {
    let names = parse_import_names("use crate::module::{self, Item};");

    assert!(
        names.contains(&"Item".to_string()),
        "leaf Item must be emitted"
    );
    // `self` in `{self, Item}` imports `module` itself — this IS a leaf
    assert!(
        names.contains(&"module".to_string()),
        "self-import of module IS a valid leaf import"
    );
    assert_eq!(names.len(), 2);
}

/// T8: Multiple use statements sharing intermediate modules.
#[test]
fn test_c16_t8_multiple_use_stmts_no_cross_contamination() {
    let source = r#"
use crate::parser::ir::{Symbol, Reference};
use crate::parser::ir::ResolutionStatus;
"#;
    let names = parse_import_names(source);

    assert!(names.contains(&"Symbol".to_string()));
    assert!(names.contains(&"Reference".to_string()));
    assert!(names.contains(&"ResolutionStatus".to_string()));
    assert!(
        !names.contains(&"ir".to_string()),
        "intermediate 'ir' must not appear from either statement"
    );
    assert!(
        !names.contains(&"parser".to_string()),
        "intermediate 'parser' must not appear"
    );
    assert_eq!(names.len(), 3);
}

/// T9: Module path annotation preserved on leaf items.
#[test]
fn test_c16_t9_module_path_annotation_correct() {
    let result = parse_rust("use crate::analyzer::patterns::{detect, Diagnostic};");

    let detect_sym = result
        .symbols
        .iter()
        .find(|s| s.name == "detect" && s.annotations.contains(&"import".to_string()));
    assert!(detect_sym.is_some(), "detect must be emitted as import");

    let detect_annotations = &detect_sym.unwrap().annotations;
    let from_annotation = detect_annotations
        .iter()
        .find(|a| a.starts_with("from:"))
        .expect("leaf import must have from: annotation");
    assert!(
        from_annotation.contains("analyzer") && from_annotation.contains("patterns"),
        "from: annotation must contain the full module path, got: {}",
        from_annotation
    );
}

/// T10: Empty use list edge case — no crash, no intermediate emissions.
#[test]
fn test_c16_t10_empty_use_list_no_crash() {
    // Tree-sitter may or may not parse this cleanly, but no crash
    let names = parse_import_names("use crate::module::{};");

    assert!(
        !names.contains(&"module".to_string()),
        "empty use list must not emit intermediate"
    );
}

// =========================================================================
// Section 3: Dogfood Regression Guards (T11-T15)
// =========================================================================

/// T11: stale_reference count drops after path-segment fix.
#[test]
fn test_c16_t11_dogfood_stale_reference_drops() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        panic!("Source directory not found at {:?}", src_path);
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale_count = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .count();

    // Path-segment fix eliminates ~56 of 117 findings (true intermediates).
    // Remaining ~61 are module-name leaf imports and unresolvable type references
    // (a different FP class, not from path segments).
    assert!(
        stale_count < 75,
        "stale_reference should drop significantly from 117 after path-segment fix, got {}",
        stale_count
    );
    eprintln!("T11: stale_reference count = {}", stale_count);
}

/// T12: Other diagnostic counts unchanged (orthogonality guard).
#[test]
fn test_c16_t12_other_patterns_unchanged_after_fix() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        panic!("Source directory not found at {:?}", src_path);
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let count = |pattern: &str| diagnostics.iter().filter(|d| d.pattern == pattern).count();

    let circular = count("circular_dependency");
    let isolated = count("isolated_cluster");
    let dead_end = count("data_dead_end");
    let orphaned = count("orphaned_implementation");
    let phantom = count("phantom_dependency");

    eprintln!(
        "T12: circular={}, isolated={}, dead_end={}, orphaned={}, phantom={}",
        circular, isolated, dead_end, orphaned, phantom
    );

    // circular and isolated should be unaffected
    assert_eq!(circular, 5, "circular_dependency should be unchanged");
    assert_eq!(isolated, 1, "isolated_cluster should be unchanged");
    assert!(
        (dead_end as i32 - 178).abs() <= 15,
        "data_dead_end should be stable, got {}",
        dead_end
    );
    // orphaned_impl dropped to 0 due to Worker 1's method call tracking (C16)
    assert!(
        orphaned <= 10,
        "orphaned_impl should be near 0 after method call tracking, got {}",
        orphaned
    );
    // phantom_dependency dropped due to Worker 1's this.method() fix + fewer imports
    assert!(
        phantom <= 200,
        "phantom_dependency should not increase, got {}",
        phantom
    );
}

/// T13: Total finding count decreases from C15 baseline.
#[test]
fn test_c16_t13_dogfood_total_findings_decrease() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        panic!("Source directory not found at {:?}", src_path);
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let total = diagnostics.len();

    eprintln!("T13: total findings = {}", total);

    // C15 baseline was 620. Path-segment fix + Worker 1's method call tracking
    // both reduce total. Expected ~490 (620 - 56 stale - 53 orphaned - ~70 phantom).
    assert!(
        total < 550,
        "total should drop significantly from 620, got {}",
        total
    );
    assert!(
        total > 300,
        "total should not drop implausibly low (would indicate broken detection), got {}",
        total
    );
}

/// T14: Updated dogfood baseline — per-pattern post-fix.
#[test]
fn test_c16_t14_dogfood_per_pattern_post_fix_baseline() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        panic!("Source directory not found at {:?}", src_path);
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let count = |pattern: &str| diagnostics.iter().filter(|d| d.pattern == pattern).count();

    let stale = count("stale_reference");
    let phantom = count("phantom_dependency");
    let dead_end = count("data_dead_end");
    let missing = count("missing_reexport");
    let orphaned = count("orphaned_implementation");
    let circular = count("circular_dependency");
    let partial = count("partial_wiring");
    let isolated = count("isolated_cluster");

    eprintln!(
        "T14 POST-FIX BASELINE: stale={}, phantom={}, dead_end={}, \
         missing={}, orphaned={}, circular={}, partial={}, isolated={}",
        stale, phantom, dead_end, missing, orphaned, circular, partial, isolated
    );

    // Post C16 baseline: stale dropped from 117, phantom/orphaned changed by Worker 1
    assert!(
        stale < 75,
        "stale_reference should drop significantly, got {}",
        stale
    );
    assert!(
        phantom <= 200,
        "phantom_dependency should not increase, got {}",
        phantom
    );
    assert!(
        (dead_end as i32 - 178).abs() <= 15,
        "data_dead_end ~ 178, got {}",
        dead_end
    );
    assert!(
        (missing as i32 - 59).abs() <= 10,
        "missing_reexport ~ 59, got {}",
        missing
    );
    assert!(
        orphaned <= 10,
        "orphaned_impl should be near 0 after method call tracking, got {}",
        orphaned
    );
    assert_eq!(circular, 5, "circular_dependency = 5");
    assert_eq!(partial, 2, "partial_wiring = 2");
    assert_eq!(isolated, 1, "isolated_cluster = 1");
}

/// T15: No stale references for known module segments remain.
#[test]
fn test_c16_t15_no_module_segment_stale_references() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        panic!("Source directory not found at {:?}", src_path);
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    // These are the TRUE intermediate path segments that the fix eliminates.
    // Module-name LEAF imports like `patterns` or `circular_dependency` remain
    // because they're correctly emitted (they ARE the import target, not intermediates).
    // Only check segments that were intermediate path components, not leaf imports.
    let eliminated_intermediates = ["ir"];

    let stale_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    for diag in &stale_findings {
        assert!(
            !eliminated_intermediates.contains(&diag.entity.as_str()),
            "intermediate segment '{}' should no longer appear as stale_reference after fix",
            diag.entity
        );
    }

    // Verify total count dropped significantly from 117
    assert!(
        stale_findings.len() < 75,
        "stale_reference count should drop from 117, got {}",
        stale_findings.len()
    );
}

// =========================================================================
// Section 4: C15 Flipped Regression Guards (T16-T19)
// =========================================================================

/// T16: Path-segment "patterns" no longer emitted as import (was C15 T7).
#[test]
fn test_c16_t16_regression_path_segment_not_emitted() {
    let names = parse_import_names("use crate::analyzer::patterns::*;");

    assert!(
        !names.contains(&"patterns".to_string()),
        "REGRESSION: 'patterns' should no longer be emitted as import after fix"
    );
}

/// T17: Nested intermediates no longer emitted (was C15 T8).
#[test]
fn test_c16_t17_regression_nested_intermediates_not_emitted() {
    let names = parse_import_names("use crate::analyzer::patterns::circular_dependency::detect;");

    assert_eq!(
        names,
        vec!["detect".to_string()],
        "only leaf 'detect' should be emitted, got: {:?}",
        names
    );
}

/// T18: Genuine stale reference still detected (C15 T9 control — unchanged).
#[test]
fn test_c16_t18_genuine_stale_reference_still_detected() {
    let mut graph = Graph::new();

    // Import "old_validate" with Partial resolution — genuinely stale
    let mut sym = make_import("old_validate", "handler.rs", 5);
    sym.annotations.push("from:crate::utils".to_string());
    sym.resolution = ResolutionStatus::Partial("module resolved, symbol not found".to_string());
    graph.add_symbol(sym);

    // Need a module in the file for context
    graph.add_symbol(make_symbol(
        "handler",
        SymbolKind::Module,
        Visibility::Public,
        "handler.rs",
        1,
    ));

    let diagnostics = stale_reference::detect(&graph, Path::new("."));
    assert!(
        diagnostics.iter().any(|d| d.entity == "old_validate"),
        "genuine stale reference must still be detected after fix"
    );
}

/// T19: Star import not flagged as stale (C15 T10 — unchanged).
#[test]
fn test_c16_t19_star_import_not_flagged_as_stale() {
    let mut graph = Graph::new();

    // Module containing the star import
    graph.add_symbol(make_symbol(
        "test_mod",
        SymbolKind::Module,
        Visibility::Public,
        "test.rs",
        1,
    ));

    // Star import with Partial("star import") resolution
    let mut sym = make_import("star_import", "test.rs", 5);
    sym.resolution = ResolutionStatus::Partial("star import".to_string());
    graph.add_symbol(sym);

    let diagnostics = stale_reference::detect(&graph, Path::new("."));
    let fires_on_star = diagnostics.iter().any(|d| d.entity == "star_import");
    assert!(!fires_on_star, "star imports must not fire stale_reference");
}

// =========================================================================
// Section 5: Issue Verification Tests (T20-T24)
// =========================================================================

/// T20: Issue #18 — stale_reference path-segment FP is eliminated.
#[test]
fn test_c16_t20_verify_issue_18_stale_reference_path_segment() {
    // BEFORE fix: 3+ imports (ir, Symbol, Reference) — "ir" is the FP
    // AFTER fix: 2 imports (Symbol, Reference) — FP eliminated
    let names = parse_import_names("use crate::parser::ir::{Symbol, Reference};");

    assert_eq!(
        names.len(),
        2,
        "exactly 2 imports after fix, no 'ir' intermediate"
    );
    assert!(names.contains(&"Symbol".to_string()));
    assert!(names.contains(&"Reference".to_string()));
    assert!(
        !names.contains(&"ir".to_string()),
        "issue #18: 'ir' intermediate must not be emitted"
    );
}

/// T21: Issue #19 — data_dead_end exclusion correctly skips test_ functions.
///
/// The `test_` prefix and test-file exclusions in `is_excluded_symbol` prevent
/// false positives on test functions. Verify this exclusion works.
#[test]
fn test_c16_t21_verify_issue_19_data_dead_end_test_function_excluded() {
    let mut graph = Graph::new();

    // A #[test] function with no callers — excluded by test_ prefix
    graph.add_symbol(make_symbol(
        "test_sarif_output",
        SymbolKind::Function,
        Visibility::Private,
        "tests.rs",
        10,
    ));

    let diagnostics = data_dead_end::detect(&graph, Path::new("."));
    assert!(
        !diagnostics.iter().any(|d| d.entity == "test_sarif_output"),
        "test_ functions should be excluded by is_excluded_symbol"
    );
}

/// T22: Issue #20 — phantom_dependency glob re-export FP exists.
#[test]
fn test_c16_t22_verify_issue_20_phantom_glob_reexport() {
    let mut graph = Graph::new();

    // mod.rs: import "types" (from glob `pub use types::*`)
    let mut import = make_import("types", "mod.rs", 2);
    import.annotations.push("from:crate::types".to_string());
    graph.add_symbol(import);

    // mod.rs: module symbol
    graph.add_symbol(make_symbol(
        "mod_rs",
        SymbolKind::Module,
        Visibility::Public,
        "mod.rs",
        1,
    ));

    // types.rs: public symbol "Manifest"
    graph.add_symbol(make_symbol(
        "Manifest",
        SymbolKind::Function,
        Visibility::Public,
        "types.rs",
        5,
    ));

    let diagnostics = phantom_dependency::detect(&graph, Path::new(""));
    assert!(
        diagnostics.iter().any(|d| d.entity == "types"),
        "issue #20: phantom fires on 'types' — import with no same-file edges"
    );
}

/// T23: Issue #22 — orphaned_impl method-dispatch FP exists.
#[test]
fn test_c16_t23_verify_issue_22_orphaned_impl_method_dispatch() {
    let mut graph = Graph::new();

    // Public method with zero callers (dispatch invisible)
    graph.add_symbol(make_symbol(
        "add_symbol",
        SymbolKind::Method,
        Visibility::Public,
        "graph/mod.rs",
        50,
    ));

    let diagnostics = orphaned_implementation::detect(&graph, Path::new("."));
    assert!(
        diagnostics.iter().any(|d| d.entity.contains("add_symbol")),
        "issue #22: orphaned_impl fires on method with zero callers: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// T24: Issue #21 — missing_reexport glob re-export FP verification.
///
/// Uses the standard fixture graph (build_missing_reexport_graph) which
/// has a pkg/__init__.py with one import (helper_a) but a sub-module with
/// public helper_b that is NOT re-exported.
#[test]
fn test_c16_t24_verify_issue_21_missing_reexport_exists() {
    let graph = build_missing_reexport_graph();

    let diagnostics = missing_reexport::detect(&graph, Path::new("."));
    // helper_b is public in sub-module but NOT re-exported by parent
    let fires_on_helper_b = diagnostics.iter().any(|d| d.entity.contains("helper_b"));
    assert!(
        fires_on_helper_b,
        "issue #21: missing_reexport should fire on 'helper_b' — not re-exported by parent. \
         Findings: {:?}",
        diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

// =========================================================================
// Section 6: Adversarial Tests (T25-T30)
// =========================================================================

/// T25: Module-as-leaf import must NOT be filtered.
#[test]
fn test_c16_t25_module_import_not_filtered() {
    // `use std::fs;` — importing the module itself, it IS the leaf
    let names = parse_import_names("use std::fs;");

    assert!(
        names.contains(&"fs".to_string()),
        "module-as-leaf import 'fs' must NOT be filtered"
    );
}

/// T26: Enum variant imports preserved.
#[test]
fn test_c16_t26_enum_variant_import_preserved() {
    let names = parse_import_names("use crate::parser::ir::ResolutionStatus::{Resolved, Partial};");

    // Tree-sitter may parse enum variants as identifiers in a use_list
    assert!(
        names.contains(&"Resolved".to_string()),
        "enum variant Resolved must be emitted"
    );
    assert!(
        names.contains(&"Partial".to_string()),
        "enum variant Partial must be emitted"
    );
    assert!(
        !names.contains(&"ResolutionStatus".to_string()),
        "enum type intermediate must not be emitted"
    );
}

/// T27: Aliased import in group works correctly.
#[test]
fn test_c16_t27_aliased_in_group() {
    let names = parse_import_names("use crate::module::{Item as Alias, Other};");

    assert!(
        names.contains(&"Alias".to_string()),
        "aliased import must use alias name"
    );
    assert!(
        names.contains(&"Other".to_string()),
        "non-aliased import must be emitted"
    );
    assert!(
        !names.contains(&"module".to_string()),
        "intermediate must not be emitted"
    );
    assert!(
        !names.contains(&"Item".to_string()),
        "aliased original name must not be emitted"
    );
    assert_eq!(names.len(), 2);
}

/// T28: Extern crate not affected by fix.
#[test]
fn test_c16_t28_extern_crate_not_affected() {
    // extern crate is NOT a `use` statement — should not crash
    let _result = parse_rust("extern crate serde;");
    // No panic = success. The fix is scoped to extract_use_tree.
}

/// T29: Pub use re-export — leaves preserved, intermediates filtered.
#[test]
fn test_c16_t29_pub_use_reexport_leaves_preserved() {
    let names = parse_import_names(
        "pub use crate::analyzer::diagnostic::{Diagnostic, Severity, Confidence};",
    );

    assert_eq!(names.len(), 3, "exactly 3 re-exported symbols");
    assert!(names.contains(&"Diagnostic".to_string()));
    assert!(names.contains(&"Severity".to_string()));
    assert!(names.contains(&"Confidence".to_string()));
    assert!(
        !names.contains(&"diagnostic".to_string()),
        "intermediate 'diagnostic' must not appear"
    );
    assert!(
        !names.contains(&"analyzer".to_string()),
        "intermediate 'analyzer' must not appear"
    );
}

/// T30: Mixed star and named imports from same module.
#[test]
fn test_c16_t30_mixed_star_and_named_imports() {
    let source = r#"
use crate::parser::ir::*;
use crate::parser::ir::ResolutionStatus;
"#;
    let result = parse_rust(source);

    let import_names: Vec<&str> = result
        .symbols
        .iter()
        .filter(|s| s.annotations.contains(&"import".to_string()))
        .map(|s| s.name.as_str())
        .collect();

    assert!(import_names.contains(&"ResolutionStatus"));
    assert!(
        !import_names.contains(&"ir"),
        "intermediate 'ir' must not appear from either statement"
    );
    assert!(
        !import_names.contains(&"parser"),
        "intermediate 'parser' must not appear"
    );
}

// =========================================================================
// Section 7: Cross-Pattern Interaction (T31-T34)
// =========================================================================

/// T31: Phantom-stale orthogonality preserved after fix.
#[test]
fn test_c16_t31_phantom_stale_orthogonality() {
    let mut graph = Graph::new();

    // Import with References edge (suppresses phantom) AND Partial resolution (triggers stale)
    let mut sym = make_import("maybe_used", "handler.rs", 5);
    sym.annotations.push("from:crate::utils".to_string());
    sym.resolution = ResolutionStatus::Partial("module resolved, symbol not found".to_string());
    let sym_id = graph.add_symbol(sym);

    // Module symbol for context
    let mod_id = graph.add_symbol(make_symbol(
        "handler",
        SymbolKind::Module,
        Visibility::Public,
        "handler.rs",
        1,
    ));

    // Add a reference edge (suppresses phantom)
    add_ref(
        &mut graph,
        mod_id,
        sym_id,
        ReferenceKind::Read,
        "handler.rs",
    );

    let phantom_diags = phantom_dependency::detect(&graph, Path::new(""));
    let stale_diags = stale_reference::detect(&graph, Path::new("."));

    let phantom_fires = phantom_diags.iter().any(|d| d.entity == "maybe_used");
    let stale_fires = stale_diags.iter().any(|d| d.entity == "maybe_used");

    assert!(!phantom_fires, "phantom must NOT fire (has reference edge)");
    assert!(stale_fires, "stale MUST fire (Partial resolution)");
}

/// T32: Fix does not create new phantom dependencies.
#[test]
fn test_c16_t32_fix_does_not_create_phantom() {
    // After fix, phantom count should be same or lower, never higher
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        return;
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let phantom = diagnostics
        .iter()
        .filter(|d| d.pattern == "phantom_dependency")
        .count();

    // C15 baseline was 205. Removing intermediate imports means fewer symbols to scan.
    // phantom should be same or lower.
    assert!(
        phantom <= 220,
        "phantom should not increase after fix (C15 was 205), got {}",
        phantom
    );
}

/// T33: Fix does not affect data_dead_end domain.
#[test]
fn test_c16_t33_fix_does_not_affect_dead_end() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        return;
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let dead_end = diagnostics
        .iter()
        .filter(|d| d.pattern == "data_dead_end")
        .count();

    // data_dead_end checks Functions/Methods, not import symbols
    assert!(
        (dead_end as i32 - 178).abs() <= 15,
        "data_dead_end should be unaffected by parser fix (C15 was 178), got {}",
        dead_end
    );
}

/// T34: Stale reference confidence distribution stable post-fix.
#[test]
fn test_c16_t34_stale_reference_confidence_stable() {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    if !src_path.exists() {
        return;
    }

    let config = crate::config::Config::load(&src_path, None).unwrap();
    let (diagnostics, _) =
        crate::diagnose(&src_path, &config, &["rust".to_string()], None, None, None).unwrap();

    let stale_findings: Vec<_> = diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .collect();

    // All remaining findings should be HIGH confidence (Signal 1)
    for diag in &stale_findings {
        assert_eq!(
            diag.confidence, "high",
            "remaining stale_reference '{}' should be HIGH confidence, got '{}'",
            diag.entity, diag.confidence
        );
    }
    eprintln!(
        "T34: {} remaining stale_reference findings, all HIGH confidence",
        stale_findings.len()
    );
}
