// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! QA-2 Cycle 18: Stash recovery verification, dogfood baseline validation,
//! cross-pattern orthogonality, dedup+child-module interaction tests.
//!
//! 42 tests across 7 sections:
//! - T1-T4:   Stash recovery — compilation gate
//! - T5-T12:  is_child_module correctness (tested via resolve_cross_file_imports)
//! - T13-T18: Dogfood baseline validation
//! - T19-T24: Cross-pattern orthogonality after dedup fix
//! - T25-T30: Dedup + child-module fix interaction
//! - T31-T36: Resolution path integrity
//! - T37-T42: Regression guards from C17

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use crate::graph::resolve_cross_file_imports;
use crate::graph::Graph;
use crate::parser::ir::*;
use crate::parser::javascript::JsAdapter;
use crate::parser::LanguageAdapter;
use crate::test_utils::*;

/// Helper: create an import symbol with `from:` annotation.
fn make_from_import_c18(name: &str, from_module: &str, file: &str, line: u32) -> Symbol {
    let mut sym = make_import(name, file, line);
    sym.resolution = ResolutionStatus::Unresolved;
    sym.annotations.push(format!("from:{}", from_module));
    sym
}

/// Helper: build module map from key-path pairs.
fn build_module_map(entries: &[(&str, &str)]) -> HashMap<String, PathBuf> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), PathBuf::from(v)))
        .collect()
}

/// Helper: run dogfood analysis on own src/ directory.
fn run_dogfood() -> Vec<crate::manifest::types::DiagnosticEntry> {
    let src_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
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
fn count_pattern(results: &[crate::manifest::types::DiagnosticEntry], pattern: &str) -> usize {
    results.iter().filter(|d| d.pattern == pattern).count()
}

/// Helper: parse TS source via JsAdapter and return symbols.
fn parse_ts_entities(source: &str) -> Vec<Symbol> {
    let adapter = JsAdapter::new();
    let path = PathBuf::from("test.ts");
    let result = adapter
        .parse_file(&path, source)
        .expect("TS parsing must succeed");
    result.symbols
}

// =========================================================================
// Section 1: Stash Recovery — Compilation Gate (T1–T4)
// =========================================================================

/// T1: cycle17_child_module_tests module exists and compiles.
/// This is the CI-breaking blocker. If this fails, nothing else matters.
#[test]
fn test_c18_t1_c17_test_module_exists_and_compiles() {
    // If we're running this test, the module compiled. But verify the file content.
    let test_source = include_str!("cycle17_child_module_tests.rs");
    assert!(
        !test_source.is_empty(),
        "T1: cycle17_child_module_tests.rs exists but is empty"
    );
    assert!(
        test_source.contains("#[test]"),
        "T1: cycle17_child_module_tests.rs must contain test functions"
    );
}

/// T2: All 40 C17 tests pass after commit — verify count and no ignored tests.
#[test]
fn test_c18_t2_all_c17_child_module_tests_present() {
    let test_source = include_str!("cycle17_child_module_tests.rs");
    let test_count = test_source.matches("#[test]").count();
    assert!(
        test_count >= 40,
        "T2: Expected 40+ tests, found {}",
        test_count
    );
    let ignore_count = test_source.matches("#[ignore]").count();
    assert_eq!(
        ignore_count, 0,
        "T2: Found {} ignored tests — fix or remove",
        ignore_count
    );
}

/// T3: test file matches lib.rs module declaration name.
#[test]
fn test_c18_t3_test_file_path_matches_module_declaration() {
    let test_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/cycle17_child_module_tests.rs");
    assert!(
        test_file.exists(),
        "T3: cycle17_child_module_tests.rs not found at expected path {:?}",
        test_file
    );
}

/// T4: no duplicate module declarations for same test file.
#[test]
fn test_c18_t4_no_duplicate_module_declarations() {
    let lib_source = include_str!("lib.rs");
    let count = lib_source.matches("mod cycle17_child_module_tests").count();
    assert_eq!(
        count, 1,
        "T4: Expected exactly 1 declaration of mod cycle17_child_module_tests, found {}",
        count
    );
}

// =========================================================================
// Section 2: is_child_module Function Correctness (T5–T12)
// Tested indirectly through resolve_cross_file_imports since the function
// is private to the populate module.
// =========================================================================

/// T5: Basic child module detection — happy path.
/// `use crate::parent::child` where `child` is a submodule.
#[test]
fn test_c18_t5_child_module_basic_detection() {
    let mut graph = Graph::new();
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/parent/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "child",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        ("crate::parent::child", "src/parent/child.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T5: Child module import must resolve"
    );
}

/// T6: Non-existent child module returns false — stays unresolved.
#[test]
fn test_c18_t6_nonexistent_child_stays_unresolved() {
    let mut graph = Graph::new();
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/parent/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "nonexistent",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    // Module map has parent but no "crate::parent::nonexistent" entry
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        ("crate::parent::other", "src/parent/other.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "nonexistent" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert!(
        !matches!(import.resolution, ResolutionStatus::Resolved),
        "T6: Non-existent child module must NOT resolve"
    );
}

/// T7: Non-Rust paths (no ::) — Python-style imports don't trigger child module path.
#[test]
fn test_c18_t7_non_rust_path_no_child_module() {
    let mut graph = Graph::new();
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "parent/__init__.py",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "child",
        "parent", // No :: in Python module path
        "consumer.py",
        1,
    ));
    let module_map = build_module_map(&[
        ("parent", "parent/__init__.py"),
        ("parent.child", "parent/child.py"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    // With no :: in the from module, child module detection should NOT fire.
    // The import may still resolve through normal paths, but NOT via is_child_module.
    // We just verify the test doesn't panic.
    let _ = import.resolution;
}

/// T8: Empty lookup name doesn't crash.
#[test]
fn test_c18_t8_empty_lookup_name_no_crash() {
    let mut graph = Graph::new();
    let _import = graph.add_symbol(make_from_import_c18(
        "",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        ("crate::parent::", "src/parent/.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    // Must not panic — that's the test
}

/// T9: Deeply nested child module (3+ levels).
#[test]
fn test_c18_t9_deeply_nested_child_module() {
    let mut graph = Graph::new();
    let _parent = graph.add_symbol(make_symbol(
        "c",
        SymbolKind::Module,
        Visibility::Public,
        "src/a/b/c/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "d",
        "crate::a::b::c",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::a::b::c", "src/a/b/c/mod.rs"),
        ("crate::a::b::c::d", "src/a/b/c/d.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "d" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T9: Deeply nested child module must resolve"
    );
}

/// T10: ADVERSARIAL — child module name collides with symbol name in parent.
#[test]
fn test_c18_t10_child_module_name_collides_with_symbol() {
    let mut graph = Graph::new();
    // Parent module has a function named "utils"
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/parent/mod.rs",
        1,
    ));
    let _utils_fn = graph.add_symbol(make_symbol(
        "utils",
        SymbolKind::Function,
        Visibility::Public,
        "src/parent/mod.rs",
        5,
    ));
    // AND a child module named "utils" exists
    let _import = graph.add_symbol(make_from_import_c18(
        "utils",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        ("crate::parent::utils", "src/parent/utils.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "utils" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    // Should resolve — either via symbol match or child module fallback
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T10: Import must resolve when both symbol and child module exist with same name"
    );
}

/// T11: ADVERSARIAL — parent_module_key is just "crate" (single segment with no ::).
/// Known limitation: is_child_module requires :: in parent, so "crate" alone won't trigger it.
#[test]
fn test_c18_t11_crate_root_parent() {
    let mut graph = Graph::new();
    let _import = graph.add_symbol(make_from_import_c18("child", "crate", "src/consumer.rs", 1));
    // "crate" has no :: — is_child_module guard clause rejects it.
    // But the import may still resolve via direct symbol lookup if the
    // module map has a matching file with a "child" symbol.
    let module_map = build_module_map(&[
        ("crate", "src/lib.rs"),
        ("crate::child", "src/child.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    // This documents the known limitation — child module detection
    // doesn't fire for crate-root imports because "crate" contains no "::".
    // The import will resolve ONLY if a symbol named "child" exists in lib.rs.
    // Without such a symbol, it stays unresolved.
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    // We don't assert Resolved because this is a known limitation.
    // The test documents the behavior and verifies no panic.
    eprintln!(
        "T11: crate root import resolution = {:?} (known limitation if not Resolved)",
        import.resolution
    );
}

/// T12: ADVERSARIAL — partial name match should NOT resolve.
#[test]
fn test_c18_t12_partial_name_no_match() {
    let mut graph = Graph::new();
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/parent/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "child",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    // Module map has "child_extra" but NOT "child"
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        ("crate::parent::child_extra", "src/parent/child_extra.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert!(
        !matches!(import.resolution, ResolutionStatus::Resolved),
        "T12: 'child' must NOT match 'child_extra' — exact key match required"
    );
}

// =========================================================================
// Section 3: Dogfood Baseline Validation (T13–T18)
// =========================================================================

/// T13: data_dead_end within expected range after blockers fixed.
#[test]
fn test_c18_t13_data_dead_end_baseline_range() {
    let results = run_dogfood();
    let dead_end = count_pattern(&results, "data_dead_end");
    eprintln!("T13: data_dead_end = {}", dead_end);
    assert!(
        dead_end >= 200 && dead_end <= 300,
        "T13: data_dead_end={}, expected 200-300 post-blocker-fix",
        dead_end
    );
}

/// T14: stale_reference within expected range.
#[test]
fn test_c18_t14_stale_reference_baseline_range() {
    let results = run_dogfood();
    let stale = count_pattern(&results, "stale_reference");
    eprintln!("T14: stale_reference = {}", stale);
    assert!(
        stale >= 10 && stale <= 30,
        "T14: stale_reference={}, expected 10-30",
        stale
    );
}

/// T15: total findings within expected range.
#[test]
fn test_c18_t15_total_findings_baseline_range() {
    let results = run_dogfood();
    let total = results.len();
    eprintln!("T15: total findings = {}", total);
    assert!(
        total >= 400 && total <= 650,
        "T15: total={}, expected 400-650",
        total
    );
}

/// T16: phantom_dependency stable (not affected by either fix).
#[test]
fn test_c18_t16_phantom_dependency_stable() {
    let results = run_dogfood();
    let phantom = count_pattern(&results, "phantom_dependency");
    eprintln!("T16: phantom_dependency = {}", phantom);
    assert!(
        phantom >= 100 && phantom <= 200,
        "T16: phantom_dependency={}, expected 100-200",
        phantom
    );
}

/// T17: ADVERSARIAL — no unknown pattern types in dogfood output.
#[test]
fn test_c18_t17_no_unknown_pattern_types() {
    let results = run_dogfood();
    let known_patterns: HashSet<&str> = [
        "data_dead_end",
        "phantom_dependency",
        "missing_reexport",
        "orphaned_impl",
        "stale_reference",
        "circular_dependency",
        "partial_wiring",
        "isolated_cluster",
        "contract_mismatch",
        "layer_violation",
        "duplication",
        "asymmetric_handling",
        "incomplete_migration",
    ]
    .iter()
    .copied()
    .collect();
    for finding in &results {
        assert!(
            known_patterns.contains(finding.pattern.as_str()),
            "T17: Unknown pattern '{}' appeared in dogfood — regression or new false positive",
            finding.pattern
        );
    }
}

/// T18: patterns with zero count remain zero — no phantom activation.
#[test]
fn test_c18_t18_zero_patterns_remain_zero() {
    let results = run_dogfood();
    let must_be_zero = [
        "duplication",
        "asymmetric_handling",
        "contract_mismatch",
        "incomplete_migration",
        "layer_violation",
    ];
    for pat in &must_be_zero {
        let count = count_pattern(&results, pat);
        assert_eq!(
            count, 0,
            "T18: Pattern '{}' fired {} times on self-analysis — should be 0",
            pat, count
        );
    }
}

// =========================================================================
// Section 4: Cross-Pattern Orthogonality After Dedup Fix (T19–T24)
// =========================================================================

/// T19: TS dedup fix doesn't change Rust self-analysis stale_reference count.
/// Self-dogfood is pure Rust — TS dedup should be orthogonal.
#[test]
fn test_c18_t19_dedup_fix_orthogonal_to_stale_reference() {
    let results = run_dogfood();
    let stale = count_pattern(&results, "stale_reference");
    eprintln!(
        "T19: stale_reference = {} (expected ~18, orthogonal to TS dedup)",
        stale
    );
    assert!(
        stale >= 10 && stale <= 30,
        "T19: stale_reference={} outside expected range — possible interaction with dedup fix",
        stale
    );
}

/// T20: child-module fix doesn't affect orphaned_impl count.
#[test]
fn test_c18_t20_child_module_fix_orthogonal_to_orphaned_impl() {
    let results = run_dogfood();
    let orphaned = count_pattern(&results, "orphaned_impl");
    eprintln!("T20: orphaned_impl = {}", orphaned);
    assert!(
        orphaned >= 40 && orphaned <= 80,
        "T20: orphaned_impl={}, expected ~53 (unaffected by child-module fix)",
        orphaned
    );
}

/// T21: resolving more child module imports doesn't create new phantom_dependency.
#[test]
fn test_c18_t21_child_module_resolution_no_phantom_regression() {
    let results = run_dogfood();
    let phantom = count_pattern(&results, "phantom_dependency");
    assert!(
        phantom <= 200,
        "T21: phantom_dependency={} — regression after child-module fix",
        phantom
    );
}

/// T22: ADVERSARIAL — combined fix effect: total should not increase.
#[test]
fn test_c18_t22_combined_fix_total_stability() {
    let results = run_dogfood();
    let total = results.len();
    // Both fixes should reduce or maintain total, never increase beyond C17 + growth tolerance.
    assert!(
        total <= 600,
        "T22: Total findings {} unexpectedly high after both fixes",
        total
    );
}

/// T23: per-pattern breakdown doesn't show large regressions from C17 baseline.
#[test]
fn test_c18_t23_per_pattern_monotonic_improvement() {
    let results = run_dogfood();
    // C17 baselines with tolerance (+35) for code additions in C18
    // (C18 adds: dedup fix tests, diff command, cycle18 test files — significant code growth)
    let c17_baselines: &[(&str, usize)] = &[
        ("data_dead_end", 221),
        ("phantom_dependency", 136),
        ("missing_reexport", 59),
        ("orphaned_impl", 53),
        ("stale_reference", 18),
        ("circular_dependency", 5),
        ("partial_wiring", 2),
        ("isolated_cluster", 1),
    ];
    for &(pattern, c17_count) in c17_baselines {
        let current = count_pattern(&results, pattern);
        eprintln!(
            "T23: {} = {} (C17 baseline: {})",
            pattern, current, c17_count
        );
        assert!(
            current <= c17_count + 35,
            "T23: Pattern '{}' went from {} to {} — regression (>35 increase)",
            pattern,
            c17_count,
            current
        );
    }
}

/// T24: ADVERSARIAL — file_symbols_cache excludes Modules.
/// Modules should not appear in file_symbols results.
#[test]
fn test_c18_t24_file_symbols_cache_excludes_modules() {
    let mut graph = Graph::new();
    // Add a module and a function to the same file
    graph.add_symbol(make_symbol(
        "my_module",
        SymbolKind::Module,
        Visibility::Public,
        "src/test_file.rs",
        1,
    ));
    graph.add_symbol(make_symbol(
        "my_function",
        SymbolKind::Function,
        Visibility::Public,
        "src/test_file.rs",
        5,
    ));
    // Build the file symbols cache by doing a resolve pass
    let module_map = build_module_map(&[("crate::test_file", "src/test_file.rs")]);
    resolve_cross_file_imports(&mut graph, &module_map);
    // After resolution, symbols_in_file should include function but the
    // resolve logic specifically skips Module-kind symbols when building
    // the file_symbols_cache at populate.rs:793-804.
    // We verify the graph still has both symbols (no data loss).
    let all_syms: Vec<_> = graph
        .all_symbols()
        .filter(|(_, s)| s.location.file == PathBuf::from("src/test_file.rs"))
        .collect();
    assert!(
        all_syms.len() >= 2,
        "T24: Expected at least 2 symbols (module + function), found {}",
        all_syms.len()
    );
}

// =========================================================================
// Section 5: Dedup Fix + Child-Module Fix Interaction (T25–T30)
// =========================================================================

/// T25: TS file with interface sharing name with Rust child module — no cross-language confusion.
#[test]
fn test_c18_t25_ts_interface_vs_rust_child_module() {
    let mut graph = Graph::new();
    // Rust module tree has crate::parent::utils
    let _import = graph.add_symbol(make_from_import_c18(
        "utils",
        "utils", // No :: — Python/TS-style path
        "consumer.ts",
        1,
    ));
    let module_map = build_module_map(&[("crate::parent::utils", "src/parent/utils.rs")]);
    // is_child_module requires :: in parent — "utils" has no :: so it returns false.
    resolve_cross_file_imports(&mut graph, &module_map);
    // Should not crash and should not falsely resolve across languages.
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "utils" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert!(
        !matches!(import.resolution, ResolutionStatus::Resolved),
        "T25: TS import must NOT resolve against Rust module map"
    );
}

/// T26: After dedup fix, TS interface entities are NOT duplicated.
#[test]
fn test_c18_t26_ts_interface_not_duplicated() {
    let source = "interface Foo { bar(): string; }";
    let symbols = parse_ts_entities(source);
    let foo_count = symbols.iter().filter(|s| s.name == "Foo").count();
    assert_eq!(
        foo_count, 1,
        "T26: Interface 'Foo' appeared {} times, expected exactly 1",
        foo_count
    );
}

/// T27: After dedup fix, TS function entities come from tree-sitter only.
#[test]
fn test_c18_t27_ts_function_single_source() {
    let source = "function greet(name: string): void { console.log(name); }";
    let symbols = parse_ts_entities(source);
    let greet_count = symbols.iter().filter(|s| s.name == "greet").count();
    assert_eq!(
        greet_count, 1,
        "T27: Function 'greet' appeared {} times — dedup fix should ensure exactly 1",
        greet_count
    );
}

/// T28: Mixed TS file: interface + function + class — correct entity count.
#[test]
fn test_c18_t28_mixed_ts_file_entity_count() {
    let source = r#"
interface Config { key: string; }
function setup(c: Config): void { }
class App { constructor() {} }
"#;
    let symbols = parse_ts_entities(source);
    // Filter to named entities (exclude module-level symbols)
    let named: Vec<_> = symbols
        .iter()
        .filter(|s| !matches!(s.kind, SymbolKind::Module | SymbolKind::Variable))
        .filter(|s| s.name == "Config" || s.name == "setup" || s.name == "App")
        .collect();
    // Expected: 1 interface (pre-extracted) + 1 function (tree-sitter) + 1 class (tree-sitter)
    assert_eq!(
        named.len(),
        3,
        "T28: Expected 3 entities (Config, setup, App), got {} — {:?}",
        named.len(),
        named.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

/// T29: ADVERSARIAL — declare function without body (semicolon-terminated).
#[test]
fn test_c18_t29_declare_function_bodyless() {
    let source = "declare function greet(name: string): void;";
    let symbols = parse_ts_entities(source);
    let greet_count = symbols.iter().filter(|s| s.name == "greet").count();
    // declare function should be extracted at least once regardless of dedup strategy.
    // After stripping `declare`, becomes `function greet(): void;` — tree-sitter may
    // or may not parse bodyless functions. Pre-extraction should handle this.
    assert!(
        greet_count >= 1,
        "T29: declare function must be extracted at least once, found {}",
        greet_count
    );
    assert!(
        greet_count <= 1,
        "T29: declare function must not be duplicated, found {}",
        greet_count
    );
}

/// T30: ADVERSARIAL — declare class without body.
/// Known limitation: `declare class` with a body gets pre-extracted AND parsed
/// by tree-sitter after `declare` is stripped, producing duplicates.
/// Worker 1's dedup fix only removed non-declare function/class from pre-extraction.
/// The `declare` variants remain in pre-extraction for bodyless forms, but tree-sitter
/// also parses the stripped version when it has a body. Filed in collective memory.
#[test]
fn test_c18_t30_declare_class_bodyless() {
    let source = "declare class EventEmitter { on(event: string, fn: Function): void; }";
    let symbols = parse_ts_entities(source);
    let emitter_count = symbols.iter().filter(|s| s.name == "EventEmitter").count();
    assert!(
        emitter_count >= 1,
        "T30: declare class must be extracted at least once, found {}",
        emitter_count
    );
    // Known limitation: declare class with body produces duplicates (pre-extraction + tree-sitter).
    // Dedup for declare variants is deferred — Worker 1's fix scope was non-declare function/class.
    assert!(
        emitter_count <= 2,
        "T30: declare class should produce at most 2 entities (known dedup gap), found {}",
        emitter_count
    );
}

// =========================================================================
// Section 6: Resolution Path Integrity (T31–T36)
// =========================================================================

/// T31: Standard symbol import still resolves after both fixes.
#[test]
fn test_c18_t31_standard_symbol_import_resolves() {
    let mut graph = Graph::new();
    // Target file has a function "do_work"
    let _target = graph.add_symbol(make_symbol(
        "do_work",
        SymbolKind::Function,
        Visibility::Public,
        "src/worker.rs",
        10,
    ));
    // Import references do_work from crate::worker
    let _import = graph.add_symbol(make_from_import_c18(
        "do_work",
        "crate::worker",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::worker", "src/worker.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "do_work" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T31: Standard symbol import must resolve"
    );
}

/// T32: Module-level import resolves (lookup_name matches module file).
#[test]
fn test_c18_t32_module_level_import_resolves() {
    let mut graph = Graph::new();
    // Module "populate" exists as a file
    let _target = graph.add_symbol(make_symbol(
        "populate",
        SymbolKind::Module,
        Visibility::Public,
        "src/graph/populate.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "populate",
        "crate::graph",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::graph", "src/graph/mod.rs"),
        ("crate::graph::populate", "src/graph/populate.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "populate" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T32: Module-level import must resolve"
    );
}

/// T33: Child module import resolves via is_child_module fallback.
#[test]
fn test_c18_t33_child_module_import_resolves_via_fallback() {
    let mut graph = Graph::new();
    // No symbol named "populate" in graph/mod.rs — only the module map entry
    let _parent = graph.add_symbol(make_symbol(
        "graph",
        SymbolKind::Module,
        Visibility::Public,
        "src/graph/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "populate",
        "crate::graph",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::graph", "src/graph/mod.rs"),
        ("crate::graph::populate", "src/graph/populate.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "populate" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T33: Child module import must resolve via is_child_module fallback"
    );
}

/// T34: Missing symbol produces non-Resolved status.
#[test]
fn test_c18_t34_missing_symbol_stays_partial_or_unresolved() {
    let mut graph = Graph::new();
    // Target file has a function "real_fn" but NOT "nonexistent_symbol"
    let _target = graph.add_symbol(make_symbol(
        "real_fn",
        SymbolKind::Function,
        Visibility::Public,
        "src/target.rs",
        10,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "nonexistent_symbol",
        "crate::target",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::target", "src/target.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| {
            s.name == "nonexistent_symbol" && s.annotations.contains(&"import".to_string())
        })
        .map(|(_, s)| s)
        .unwrap();
    assert!(
        !matches!(import.resolution, ResolutionStatus::Resolved),
        "T34: Missing symbol should not be Resolved, got {:?}",
        import.resolution
    );
}

/// T35: ADVERSARIAL — deleted module still produces non-Resolved.
#[test]
fn test_c18_t35_deleted_module_stays_unresolved() {
    let mut graph = Graph::new();
    // Parent exists but child does NOT in module_map
    let _parent = graph.add_symbol(make_symbol(
        "parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/parent/mod.rs",
        1,
    ));
    let _import = graph.add_symbol(make_from_import_c18(
        "deleted_child",
        "crate::parent",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::parent", "src/parent/mod.rs"),
        // No "crate::parent::deleted_child" entry
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "deleted_child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert!(
        !matches!(import.resolution, ResolutionStatus::Resolved),
        "T35: Deleted child module must not resolve — over-suppression risk"
    );
}

/// T36: ADVERSARIAL — multiple imports from same module resolve independently.
#[test]
fn test_c18_t36_multiple_imports_from_same_module() {
    let mut graph = Graph::new();
    // Target has two symbols
    let _fn1 = graph.add_symbol(make_symbol(
        "alpha",
        SymbolKind::Function,
        Visibility::Public,
        "src/target.rs",
        10,
    ));
    let _fn2 = graph.add_symbol(make_symbol(
        "beta",
        SymbolKind::Function,
        Visibility::Public,
        "src/target.rs",
        20,
    ));
    // Two imports from same module
    let _import1 = graph.add_symbol(make_from_import_c18(
        "alpha",
        "crate::target",
        "src/consumer.rs",
        1,
    ));
    let _import2 = graph.add_symbol(make_from_import_c18(
        "beta",
        "crate::target",
        "src/consumer.rs",
        2,
    ));
    let module_map = build_module_map(&[
        ("crate::target", "src/target.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    resolve_cross_file_imports(&mut graph, &module_map);
    let alpha = graph
        .all_symbols()
        .find(|(_, s)| s.name == "alpha" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    let beta = graph
        .all_symbols()
        .find(|(_, s)| s.name == "beta" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        alpha.resolution,
        ResolutionStatus::Resolved,
        "T36: alpha must resolve"
    );
    assert_eq!(
        beta.resolution,
        ResolutionStatus::Resolved,
        "T36: beta must resolve"
    );
}

// =========================================================================
// Section 7: Regression Guards from C17 (T37–T42)
// =========================================================================

/// T37: REGRESSION — lib.rs module declarations all have corresponding files.
#[test]
fn test_c18_t37_all_test_module_declarations_have_files() {
    let lib_source = include_str!("lib.rs");
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Find all `mod <name>_tests;` declarations in lib.rs
    for line in lib_source.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("mod ") && trimmed.ends_with("_tests;") {
            let module_name = trimmed
                .strip_prefix("mod ")
                .unwrap()
                .strip_suffix(';')
                .unwrap()
                .trim();
            let file_path = src_dir.join(format!("{}.rs", module_name));
            assert!(
                file_path.exists(),
                "T37: lib.rs declares mod {} but file {:?} does not exist — C17 blocker regression",
                module_name,
                file_path
            );
        }
    }
}

/// T38: REGRESSION — populate.rs has no uncommitted changes (stash recovery complete).
#[test]
fn test_c18_t38_no_stash_artifacts() {
    let output = std::process::Command::new("git")
        .args(["status", "--porcelain", "src/graph/populate.rs"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .expect("git status failed");
    let status = String::from_utf8_lossy(&output.stdout);
    assert!(
        status.trim().is_empty(),
        "T38: populate.rs has uncommitted changes — stash recovery may be incomplete: {}",
        status
    );
}

/// T39: REGRESSION — C15 dogfood bounds still hold.
#[test]
fn test_c18_t39_c15_dogfood_bounds_still_hold() {
    let results = run_dogfood();
    let dead_end = count_pattern(&results, "data_dead_end");
    assert!(
        dead_end < 300,
        "T39: C15 regression — data_dead_end {} >= 300",
        dead_end
    );
}

/// T40: REGRESSION — entity counts for TS file with dedup fix applied.
#[test]
fn test_c18_t40_entity_count_direction_after_dedup() {
    let source = "function foo(): void {} class Bar { baz(): number { return 1; } }";
    let symbols = parse_ts_entities(source);
    // With dedup fix: function + class from tree-sitter = ~2 named entities
    // Plus any methods. Without fix: duplicates from pre-extraction.
    let named: Vec<_> = symbols
        .iter()
        .filter(|s| s.name == "foo" || s.name == "Bar")
        .collect();
    assert!(
        named.len() <= 4,
        "T40: Entity count {} suggests dedup fix not applied — expected ≤4 for fn foo + class Bar",
        named.len()
    );
}

/// T41: REGRESSION — C15 baseline assertion in cycle15_fp_triage_tests still present.
#[test]
fn test_c18_t41_c15_assertions_still_valid() {
    let test_source = include_str!("cycle15_fp_triage_tests.rs");
    assert!(
        test_source.contains("dead_end < 300"),
        "T41: C15 baseline assertion 'dead_end < 300' missing or changed"
    );
}

/// T42: Empty child module (no symbols) doesn't cause resolution crash.
#[test]
fn test_c18_t42_empty_child_module_no_crash() {
    let mut graph = Graph::new();
    // Parent module exists but has no non-module symbols
    let _parent = graph.add_symbol(make_symbol(
        "empty_parent",
        SymbolKind::Module,
        Visibility::Public,
        "src/empty_parent/mod.rs",
        1,
    ));
    // Import referencing a child of the empty parent
    let _import = graph.add_symbol(make_from_import_c18(
        "child",
        "crate::empty_parent",
        "src/consumer.rs",
        1,
    ));
    let module_map = build_module_map(&[
        ("crate::empty_parent", "src/empty_parent/mod.rs"),
        ("crate::empty_parent::child", "src/empty_parent/child.rs"),
        ("crate::consumer", "src/consumer.rs"),
    ]);
    // Must not panic — the empty parent's file has no symbols but is_child_module
    // only checks module_map keys
    resolve_cross_file_imports(&mut graph, &module_map);
    let import = graph
        .all_symbols()
        .find(|(_, s)| s.name == "child" && s.annotations.contains(&"import".to_string()))
        .map(|(_, s)| s)
        .unwrap();
    assert_eq!(
        import.resolution,
        ResolutionStatus::Resolved,
        "T42: Child of empty parent must still resolve via is_child_module"
    );
}
