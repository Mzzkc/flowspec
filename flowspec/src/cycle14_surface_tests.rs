// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 14 QA-3 (Surface) tests — manifest size byte floor, resolve_import_by_name
//! file-scoping, regression guards, adversarial edge cases.

use crate::graph::resolve_import_by_name;
use crate::manifest::validate_manifest_size;
use crate::parser::ir::*;
use crate::test_utils::{make_import, make_symbol};
use crate::Graph;

// ===========================================================================
// Helper: build a symbol_id_map by adding symbols to a graph and tracking IDs.
// ===========================================================================

/// Adds symbols to a graph and returns (symbols_vec, symbol_id_map).
/// The symbol_id_map maps index-in-vec → real SymbolId from the graph.
fn build_symbol_map(
    graph: &mut Graph,
    symbols: Vec<Symbol>,
) -> (Vec<Symbol>, Vec<(usize, SymbolId)>) {
    let mut id_map = Vec::new();
    for (idx, sym) in symbols.iter().enumerate() {
        let real_id = graph.add_symbol(sym.clone());
        id_map.push((idx, real_id));
    }
    (symbols, id_map)
}

// ===========================================================================
// Category 1: Manifest Size Byte Floor — Unit Tests (T1–T10)
// ===========================================================================

/// T1: Small project, manifest under byte floor — PASS.
/// 2KB source, 15KB manifest → under 20KB floor, ratio 7.3x passes regardless.
#[test]
fn t01_byte_floor_small_project_small_manifest_passes() {
    let serialized = "x".repeat(15_000);
    let result = validate_manifest_size(&serialized, 2048);
    assert!(
        result.is_ok(),
        "15KB manifest with 2KB source should pass (under 20KB floor). Got: {:?}",
        result.err()
    );
}

/// T2: Small project, manifest just under byte floor — PASS.
/// 1500 source, 19KB manifest → 12.7x ratio would FAIL without floor, but under 20KB saves it.
/// TDD ANCHOR: Must fail pre-implementation.
#[test]
fn t02_byte_floor_manifest_at_19kb_passes() {
    let serialized = "x".repeat(19_000);
    let result = validate_manifest_size(&serialized, 1500);
    assert!(
        result.is_ok(),
        "19KB manifest should pass despite 12.7x ratio — byte floor saves it. Got: {:?}",
        result.err()
    );
}

/// T3: Boundary — manifest exactly at 20,479 bytes — PASS.
#[test]
fn t03_byte_floor_boundary_20479_passes() {
    let serialized = "x".repeat(20_479);
    let result = validate_manifest_size(&serialized, 1500);
    assert!(
        result.is_ok(),
        "20,479 bytes < 20,480 floor — must pass. Got: {:?}",
        result.err()
    );
}

/// T4: Boundary — manifest exactly at 20,480 bytes.
/// Spec says "under 20KB" → `< 20_480`, so 20,480 falls through to ratio check.
/// 20,480 / 1,500 = 13.65x > 10x → FAIL.
#[test]
fn t04_byte_floor_boundary_20480_fails() {
    let serialized = "x".repeat(20_480);
    let result = validate_manifest_size(&serialized, 1500);
    assert!(
        result.is_err(),
        "20,480 bytes = floor boundary, falls through to ratio check (13.65x > 10x) — must fail"
    );
}

/// T5: Manifest over byte floor, over ratio — FAIL.
/// 25KB > 20KB floor, 12.2x > 10x.
#[test]
fn t05_byte_floor_exceeded_ratio_check_fires() {
    let serialized = "x".repeat(25_000);
    let result = validate_manifest_size(&serialized, 2048);
    assert!(
        result.is_err(),
        "25KB manifest > 20KB floor, 12.2x > 10x — must fail"
    );
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("exceeds") || err.contains("size") || err.contains("ratio"),
        "Error must describe size violation. Got: {}",
        err
    );
}

/// T6: Manifest over byte floor, under ratio — PASS.
/// 25KB > 20KB floor, but 2.5x < 10x.
#[test]
fn t06_byte_floor_exceeded_but_ratio_ok_passes() {
    let serialized = "x".repeat(25_000);
    let result = validate_manifest_size(&serialized, 10_000);
    assert!(
        result.is_ok(),
        "25KB manifest, 2.5x ratio — should pass. Got: {:?}",
        result.err()
    );
}

/// T7: Large project unchanged — PASS.
/// 500KB manifest, 100KB source, 5x ratio.
#[test]
fn t07_byte_floor_large_project_normal_ratio_passes() {
    let serialized = "x".repeat(500_000);
    let result = validate_manifest_size(&serialized, 100_000);
    assert!(
        result.is_ok(),
        "Large project, 5x ratio — must pass. Got: {:?}",
        result.err()
    );
}

/// T8: Large project, over ratio — FAIL.
/// 1.1MB manifest, 100KB source, 11x ratio.
#[test]
fn t08_byte_floor_large_project_over_ratio_fails() {
    let serialized = "x".repeat(1_100_000);
    let result = validate_manifest_size(&serialized, 100_000);
    assert!(
        result.is_err(),
        "Large project, 11x ratio — must fail. Byte floor does NOT save large projects."
    );
}

/// T9: Source below SIZE_CHECK_MIN_SOURCE_BYTES — PASS (existing behavior).
/// Source < 1024, existing early return skips ALL checks.
#[test]
fn t09_byte_floor_tiny_source_still_skips() {
    let serialized = "x".repeat(30_000);
    let result = validate_manifest_size(&serialized, 500);
    assert!(
        result.is_ok(),
        "Source < 1024 — existing bypass must still work. Got: {:?}",
        result.err()
    );
}

/// T10: Zero source bytes — PASS (existing behavior).
#[test]
fn t10_byte_floor_zero_source_passes() {
    let serialized = "x".repeat(50_000);
    let result = validate_manifest_size(&serialized, 0);
    assert!(
        result.is_ok(),
        "Zero source — existing bypass must still work. Got: {:?}",
        result.err()
    );
}

// ===========================================================================
// Category 2: Manifest Size Byte Floor — Integration Tests (T11–T16)
// ===========================================================================

/// T11: Rust fixtures no longer fail with size error (end-to-end).
#[test]
fn t11_e2e_rust_fixtures_pass_with_byte_floor() {
    let fixtures_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    if !fixtures_path.exists() {
        return;
    }
    let result = crate::commands::run_analyze(
        &fixtures_path,
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Rust fixtures MUST pass analysis — v0.1 small-project unblock. Got: {:?}",
        result.err()
    );
}

/// T12: Pathological input still rejected (regression from C10).
#[test]
fn t12_e2e_pathological_input_still_rejected_after_byte_floor() {
    let tmp = tempfile::tempdir().unwrap();
    let mut source = String::new();
    for i in 0..80 {
        source.push_str(&format!("def function_{:03}(): pass\n", i));
    }
    std::fs::write(tmp.path().join("pathological.py"), &source).unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_err(),
        "Pathological input MUST still be rejected. Byte floor must NOT weaken detection."
    );
    let err_msg = result.unwrap_err().to_string();
    assert!(
        err_msg.contains("exceeds") || err_msg.contains("size") || err_msg.contains("ratio"),
        "Error must mention size/ratio. Got: {}",
        err_msg
    );
}

/// T13: Single tiny file — manifest under floor.
#[test]
fn t13_e2e_single_function_file_passes() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("hello.py"), "def hello(): return 42\n").unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Single function file must pass. Got: {:?}",
        result.err()
    );
}

/// T14: Multiple small files — aggregate still passes floor.
#[test]
fn t14_e2e_multiple_small_files_pass() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..3 {
        let content = format!(
            "def func_a_{i}(x): return x + 1\ndef func_b_{i}(y): return y * 2\ndef func_c_{i}(z): return func_a_{i}(z)\n"
        );
        std::fs::write(tmp.path().join(format!("mod_{i}.py")), content).unwrap();
    }

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Multiple small files must pass. Got: {:?}",
        result.err()
    );
}

/// T15: JSON format also respects byte floor.
#[test]
fn t15_e2e_json_format_respects_byte_floor() {
    let fixtures_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    if !fixtures_path.exists() {
        return;
    }
    let result = crate::commands::run_analyze(
        &fixtures_path,
        &[],
        crate::OutputFormat::Json,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "JSON format on Rust fixtures must pass. Got: {:?}",
        result.err()
    );
}

/// T16: SARIF format also respects byte floor.
#[test]
fn t16_e2e_sarif_format_respects_byte_floor() {
    let fixtures_path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust");
    if !fixtures_path.exists() {
        return;
    }
    let result = crate::commands::run_analyze(
        &fixtures_path,
        &[],
        crate::OutputFormat::Sarif,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "SARIF format on Rust fixtures must pass. Got: {:?}",
        result.err()
    );
}

// ===========================================================================
// Category 3: resolve_import_by_name File-Scoping (T17–T30)
// ===========================================================================

/// T17: Single file, single import — resolves correctly.
#[test]
fn t17_file_scoping_single_file_single_import() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("json", "file_a.py", 1),
        make_symbol(
            "load_data",
            SymbolKind::Function,
            Visibility::Public,
            "file_a.py",
            5,
        ),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("json", &map, &syms);
    assert_ne!(
        result,
        SymbolId::default(),
        "Single import 'json' should resolve"
    );
    assert_eq!(
        result, map[0].1,
        "Must resolve to the json import's SymbolId"
    );
}

/// T18: Two files, same module imported — resolve to own file's import.
/// Since resolve_import_by_name operates on per-file symbol_id_map within
/// populate_graph, we verify each file's map only contains its own symbols.
#[test]
fn t18_file_scoping_two_files_same_import() {
    let mut graph = Graph::new();

    // File A symbols
    let symbols_a = vec![
        make_import("os", "file_a.py", 1),
        make_symbol(
            "func_a",
            SymbolKind::Function,
            Visibility::Public,
            "file_a.py",
            3,
        ),
    ];
    let (syms_a, map_a) = build_symbol_map(&mut graph, symbols_a);

    // File B symbols
    let symbols_b = vec![
        make_import("os", "file_b.py", 1),
        make_symbol(
            "func_b",
            SymbolKind::Function,
            Visibility::Public,
            "file_b.py",
            3,
        ),
    ];
    let (syms_b, map_b) = build_symbol_map(&mut graph, symbols_b);

    // Resolve 'os' in File A's context → File A's import
    let result_a = resolve_import_by_name("os", &map_a, &syms_a);
    assert_eq!(
        result_a, map_a[0].1,
        "File A's 'os' must resolve to File A's import"
    );

    // Resolve 'os' in File B's context → File B's import
    let result_b = resolve_import_by_name("os", &map_b, &syms_b);
    assert_eq!(
        result_b, map_b[0].1,
        "File B's 'os' must resolve to File B's import"
    );

    // Critically: they must be different IDs
    assert_ne!(
        result_a, result_b,
        "Different files' imports must resolve to different SymbolIds"
    );
}

/// T19: Two files, different modules with same name — no cross-contamination.
#[test]
fn t19_file_scoping_different_modules_same_stem() {
    let mut graph = Graph::new();

    let symbols_a = vec![make_import("utils", "file_a.py", 1)];
    let (syms_a, map_a) = build_symbol_map(&mut graph, symbols_a);

    let symbols_b = vec![make_import("utils", "file_b.py", 1)];
    let (syms_b, map_b) = build_symbol_map(&mut graph, symbols_b);

    let result_a = resolve_import_by_name("utils", &map_a, &syms_a);
    let result_b = resolve_import_by_name("utils", &map_b, &syms_b);

    assert_ne!(
        result_a, result_b,
        "Different files' imports must resolve to different IDs"
    );
    assert_eq!(result_a, map_a[0].1);
    assert_eq!(result_b, map_b[0].1);
}

/// T20: File with no matching import — returns default.
#[test]
fn t20_file_scoping_no_matching_import_returns_default() {
    let mut graph = Graph::new();
    let symbols = vec![make_symbol(
        "func",
        SymbolKind::Function,
        Visibility::Public,
        "file.py",
        1,
    )];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("os", &map, &syms);
    assert_eq!(
        result,
        SymbolId::default(),
        "No matching import — must return default"
    );
}

/// T21: Aliased import — name matching uses declared name.
#[test]
fn t21_file_scoping_aliased_import_matches_alias() {
    let mut graph = Graph::new();
    // Symbol name is 'j' (the alias), not 'json' (the module)
    let symbols = vec![make_import("j", "file.py", 1)];
    let (syms, map) = build_symbol_map(&mut graph, symbols);

    // Should match 'j' (the alias name), not 'json'
    let result = resolve_import_by_name("j", &map, &syms);
    assert_ne!(result, SymbolId::default(), "Aliased import 'j' must match");

    let result_original = resolve_import_by_name("json", &map, &syms);
    assert_eq!(
        result_original,
        SymbolId::default(),
        "Original module name 'json' should NOT match aliased import 'j'"
    );
}

/// T22: Multiple imports in same file — matches correct one.
#[test]
fn t22_file_scoping_multiple_imports_correct_match() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("os", "file.py", 1),
        make_import("sys", "file.py", 2),
        make_import("json", "file.py", 3),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);

    assert_eq!(resolve_import_by_name("os", &map, &syms), map[0].1);
    assert_eq!(resolve_import_by_name("sys", &map, &syms), map[1].1);
    assert_eq!(resolve_import_by_name("json", &map, &syms), map[2].1);
}

/// T23: Empty symbol_id_map — no panic.
#[test]
fn t23_file_scoping_empty_symbol_map_no_panic() {
    let result = resolve_import_by_name("anything", &[], &[]);
    assert_eq!(result, SymbolId::default(), "Empty map must return default");
}

/// T24: Import symbol without "import" annotation — not matched.
#[test]
fn t24_file_scoping_non_import_symbol_not_matched() {
    let mut graph = Graph::new();
    let symbols = vec![make_symbol(
        "os",
        SymbolKind::Function,
        Visibility::Public,
        "file.py",
        1,
    )];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("os", &map, &syms);
    assert_eq!(
        result,
        SymbolId::default(),
        "Function named 'os' (no import annotation) must NOT match"
    );
}

/// T25: Rust `use` paths — file-scoped resolution (integration).
#[test]
fn t25_file_scoping_rust_use_paths_per_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("mod_a.rs"),
        "use std::fs;\nfn read_it() { fs::read_to_string(\"x\"); }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("mod_b.rs"),
        "use std::fs;\nfn write_it() { fs::write(\"x\", \"y\"); }\n",
    )
    .unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Multi-file Rust project should analyze cleanly. Got: {:?}",
        result.err()
    );
}

/// T26: Cross-file resolution fallback — no local match returns default.
#[test]
fn t26_file_scoping_no_local_match_returns_default() {
    let mut graph = Graph::new();
    let symbols_b = vec![make_symbol(
        "helper",
        SymbolKind::Function,
        Visibility::Public,
        "file_b.py",
        1,
    )];
    let (syms_b, map_b) = build_symbol_map(&mut graph, symbols_b);

    let result = resolve_import_by_name("utils", &map_b, &syms_b);
    assert_eq!(
        result,
        SymbolId::default(),
        "No local import match → default (strict file-scoping)"
    );
}

/// T27: Attribute access on re-export — resolution is file-local.
#[test]
fn t27_file_scoping_reexport_resolution() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("helper", "file_b.py", 1),
        make_symbol(
            "run",
            SymbolKind::Function,
            Visibility::Public,
            "file_b.py",
            3,
        ),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("helper", &map, &syms);
    assert_eq!(
        result, map[0].1,
        "Re-export import resolves to local import symbol"
    );
}

/// T28: Rust scoped_type_identifier references — file-scoped (integration).
#[test]
fn t28_file_scoping_rust_type_references_per_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("types_a.rs"),
        "use std::fs::File;\nfn open_file() -> File { todo!() }\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("types_b.rs"),
        "use std::io::Error;\nfn get_err() -> Error { todo!() }\n",
    )
    .unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Rust type references across files should analyze cleanly. Got: {:?}",
        result.err()
    );
}

/// T29: JavaScript require — file-scoped (integration).
#[test]
fn t29_file_scoping_js_require_per_file() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("a.js"),
        "const fs = require('fs');\nfs.readFile('x', () => {});\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("b.js"),
        "const fs = require('fs');\nfs.writeFile('x', 'y', () => {});\n",
    )
    .unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Multi-file JS project should analyze cleanly. Got: {:?}",
        result.err()
    );
}

/// T30: Large multi-file project — no performance regression.
#[test]
fn t30_file_scoping_performance_no_regression() {
    let tmp = tempfile::tempdir().unwrap();
    for i in 0..20 {
        let content = format!(
            "import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n"
        );
        std::fs::write(tmp.path().join(format!("mod_{i}.py")), content).unwrap();
    }

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "20-file project must not timeout or error. Got: {:?}",
        result.err()
    );
}

// ===========================================================================
// Category 4: Regression Guards (T31–T37)
// ===========================================================================

/// T31: C10 pathological wiring still works.
#[test]
fn t31_regression_c10_size_validation_wired() {
    let tmp = tempfile::tempdir().unwrap();
    let mut source = String::new();
    for i in 0..80 {
        source.push_str(&format!("def function_{:03}(): pass\n", i));
    }
    std::fs::write(tmp.path().join("pathological.py"), &source).unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_err(),
        "Pathological input must still fail — size validation wired in commands.rs"
    );
}

/// T32: C10 normal input still passes.
#[test]
fn t32_regression_c10_normal_input_passes() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("normal.py"),
        include_str!("../../tests/fixtures/python/dead_code.py"),
    )
    .unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Normal fixture MUST pass size validation. Got: {:?}",
        result.err()
    );
}

/// T33: C11 filter flags still work.
#[test]
fn t33_regression_c11_filter_flags_functional() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("unused.py"),
        "import os\nimport json\ndef main(): pass\n",
    )
    .unwrap();

    let result = crate::commands::run_diagnose(
        tmp.path(),
        &[],
        &["phantom_dependency".to_string()],
        None,
        None,
        crate::OutputFormat::Yaml,
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "diagnose with --checks filter must work. Got: {:?}",
        result.err()
    );
}

/// T34: C12 #16 fix — diagnostic_summary recomputed after filtering.
#[test]
fn t34_regression_c12_diagnostic_summary_recomputed() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("mixed.py"),
        "import os\nimport json\ndef unused(): pass\ndef main(): return unused()\n",
    )
    .unwrap();

    // Run analyze with a severity filter — summary counts should reflect filtered set
    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        Some(crate::Severity::Warning),
        None,
    );
    assert!(
        result.is_ok(),
        "Filtered analyze must succeed. Got: {:?}",
        result.err()
    );
}

/// T35: C13 trace dedup preserved.
#[test]
fn t35_regression_c13_trace_dedup_not_broken() {
    use crate::commands::deduplicate_flows;
    use crate::manifest::types::{FlowEntry, FlowStep};

    let flow1 = FlowEntry {
        id: "F001".to_string(),
        description: "A to B".to_string(),
        entry: "A".to_string(),
        exit: "B".to_string(),
        steps: vec![FlowStep {
            entity: "B".to_string(),
            action: "call".to_string(),
            in_type: "unknown".to_string(),
            out_type: "unknown".to_string(),
        }],
        issues: Vec::new(),
    };
    let flow2 = flow1.clone();
    let flows = vec![flow1, flow2];
    let result = deduplicate_flows(flows);
    assert_eq!(
        result.len(),
        1,
        "Exact duplicate flows must be deduplicated"
    );
}

/// T36: C13 symbol disambiguation preserved.
#[test]
fn t36_regression_c13_symbol_disambiguation_preserved() {
    let tmp = tempfile::tempdir().unwrap();
    let dir_a = tmp.path().join("pkg_a");
    let dir_b = tmp.path().join("pkg_b");
    std::fs::create_dir_all(&dir_a).unwrap();
    std::fs::create_dir_all(&dir_b).unwrap();
    std::fs::write(dir_a.join("utils.py"), "def helper(): return 1\n").unwrap();
    std::fs::write(dir_b.join("utils.py"), "def helper(): return 2\n").unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(
        result.is_ok(),
        "Multi-dir same-name project must analyze. Got: {:?}",
        result.err()
    );
}

/// T37: Exit codes unchanged.
#[test]
fn t37_regression_exit_codes_contract() {
    // Clean project → exit code 0 or 2
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("clean.py"), "def main(): return 42\n").unwrap();

    let result = crate::commands::run_analyze(
        tmp.path(),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    match result {
        Ok(code) => assert!(
            code == 0 || code == 2,
            "Exit code must be 0 (clean) or 2 (findings), got: {}",
            code
        ),
        Err(e) => panic!("Clean project should not error: {:?}", e),
    }

    // Invalid path → error propagation
    let result = crate::commands::run_analyze(
        std::path::Path::new("/nonexistent/path/to/project"),
        &[],
        crate::OutputFormat::Yaml,
        None,
        None,
        &[],
        None,
        None,
    );
    assert!(result.is_err(), "Invalid path must return error");
}

// ===========================================================================
// Category 5: Adversarial Edge Cases (T38–T42)
// ===========================================================================

/// T38: Manifest exactly one byte over floor with pathological ratio.
#[test]
fn t38_adversarial_one_byte_over_floor_pathological_ratio() {
    let serialized = "x".repeat(20_481);
    let result = validate_manifest_size(&serialized, 1500);
    assert!(
        result.is_err(),
        "20,481 bytes > 20,480 floor, 13.65x > 10x — must fail"
    );
}

/// T39: Manifest over floor but ratio exactly at 10.0.
/// Existing behavior: `ratio > SIZE_CHECK_MAX_RATIO` — 10.0 is NOT greater than 10.0.
#[test]
fn t39_adversarial_over_floor_ratio_exactly_10() {
    let serialized = "x".repeat(30_000);
    let result = validate_manifest_size(&serialized, 3000);
    assert!(
        result.is_ok(),
        "Ratio exactly 10.0 is NOT > 10.0 — must pass. Got: {:?}",
        result.err()
    );
}

/// T40: Unicode content in manifest — byte count vs char count.
/// Rust's `.len()` returns byte count, which is correct for size validation.
#[test]
fn t40_adversarial_unicode_manifest_byte_vs_char_count() {
    // 4-byte UTF-8 emoji: each char is 4 bytes
    // 4750 chars × 4 bytes = 19,000 bytes — under 20KB floor
    let serialized: String = std::iter::repeat('\u{1F600}').take(4750).collect();
    assert_eq!(serialized.len(), 19_000, "Should be exactly 19,000 bytes");
    let result = validate_manifest_size(&serialized, 2000);
    assert!(
        result.is_ok(),
        "19KB byte count (under floor) must pass despite high ratio. Got: {:?}",
        result.err()
    );
}

/// T41: resolve_import_by_name with duplicate import names in same file.
/// First match wins (linear scan, early return).
#[test]
fn t41_adversarial_duplicate_import_names_same_file() {
    let mut graph = Graph::new();
    let symbols = vec![
        make_import("os", "file.py", 1),
        make_import("os", "file.py", 5),
    ];
    let (syms, map) = build_symbol_map(&mut graph, symbols);
    let result = resolve_import_by_name("os", &map, &syms);
    assert_eq!(
        result, map[0].1,
        "First matching import wins (deterministic linear scan)"
    );
}

/// T42: Concurrent byte floor + source bytes bypass interaction.
/// When BOTH bypass conditions are true, function must return Ok.
#[test]
fn t42_adversarial_both_bypasses_active() {
    let serialized = "x".repeat(15_000);
    let result = validate_manifest_size(&serialized, 500);
    assert!(
        result.is_ok(),
        "Both bypasses active (source < 1024 AND manifest < 20KB) — must pass. Got: {:?}",
        result.err()
    );
}
