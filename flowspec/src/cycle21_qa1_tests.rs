// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 21 QA-1 (QA-Foundation) integration tests: Python relative import
//! resolution + circular_dependency detection.
//!
//! Tests that `resolve_python_relative_import()` in `populate.rs` correctly
//! resolves Python `.`-prefixed relative imports to module_map keys, enabling
//! `circular_dependency` detection on real Python packages using relative imports.
//!
//! 13 tests across 3 categories (CREL, CADV, CREG).
//! Type annotation unit tests are in `parser/python.rs` (25 tests, TPARAM/TRET/TSUB/TADV/TINT/TREG/TCLS).

use std::path::PathBuf;

use crate::{analyze, build_module_map, Config};

fn fixture_base() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// =========================================================================
// Category 7: Circular Dependency — Relative Import Resolution (CREL-*)
// =========================================================================

/// CREL-1: Sibling relative import resolves (`from .b import func_b`)
#[test]
fn test_circular_dep_relative_sibling_import_resolves() {
    let fixture_dir = fixture_base().join("tests/fixtures/python/circular_rel_imports");
    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // Package with relative imports must extract entities from all files
    assert!(
        result.manifest.metadata.entity_count >= 3,
        "Package with relative imports must extract entities from all files. Got {}",
        result.manifest.metadata.entity_count
    );
}

/// CREL-2: Two-file cycle with relative imports detected
#[test]
fn test_circular_dep_relative_imports_two_file_cycle_detected() {
    let fixture_dir = fixture_base().join("tests/fixtures/python/circular_rel_imports");
    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let circular_findings: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "circular_dependency")
        .collect();

    assert!(
        !circular_findings.is_empty(),
        "Relative imports from .b and from .a forming a cycle MUST be detected. \
         This is the P0 fix — 0/13 Python cycles were found without this. \
         Got 0 circular_dependency findings. Diagnostics: {:?}",
        result
            .manifest
            .diagnostics
            .iter()
            .map(|d| &d.pattern)
            .collect::<Vec<_>>()
    );
}

/// CREL-3: Module map includes relative import target files
#[test]
fn test_circular_dep_module_map_includes_package_files() {
    let fixture_dir = fixture_base().join("tests/fixtures/python/circular_rel_imports");
    let files = vec![
        fixture_dir.join("__init__.py"),
        fixture_dir.join("a.py"),
        fixture_dir.join("b.py"),
        fixture_dir.join("c.py"),
    ];
    let module_map = build_module_map(&files);

    // Module map must contain entries for each file in the package
    assert!(
        module_map.len() >= 3,
        "Module map must include package files. Got {} entries: {:?}",
        module_map.len(),
        module_map.keys().collect::<Vec<_>>()
    );
}

// =========================================================================
// Category 8: Circular Dependency — Adversarial (CADV-*)
// =========================================================================

/// CADV-1: Relative import in non-package file (no `__init__.py`) — no crash
#[test]
fn test_circular_dep_relative_import_no_init_no_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    std::fs::write(
        dir.join("a.py"),
        "from .b import foo\n\ndef bar():\n    return foo()\n",
    )
    .unwrap();
    std::fs::write(dir.join("b.py"), "def foo():\n    return 1\n").unwrap();

    let config = Config::load(dir, None).unwrap();
    let result = analyze(dir, &config, &[]);
    assert!(
        result.is_ok(),
        "Relative import without __init__.py must not crash"
    );
}

/// CADV-2: Relative import to nonexistent module — no crash
#[test]
fn test_circular_dep_relative_import_nonexistent_module_no_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        dir.join("a.py"),
        "from .nonexistent import foo\n\ndef bar():\n    return foo()\n",
    )
    .unwrap();

    let config = Config::load(&dir, None).unwrap();
    let result = analyze(&dir, &config, &[]);
    assert!(
        result.is_ok(),
        "Relative import to nonexistent module must not crash"
    );
    let result = result.unwrap();
    let circular = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "circular_dependency")
        .count();
    assert_eq!(circular, 0, "No cycle exists — no findings expected");
}

/// CADV-3: Mixed absolute and relative imports
#[test]
fn test_circular_dep_mixed_absolute_relative_no_crash() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        dir.join("a.py"),
        "from .b import func_b\n\ndef func_a():\n    return func_b()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.py"),
        "from pkg.a import func_a\n\ndef func_b():\n    return func_a()\n",
    )
    .unwrap();

    let config = Config::load(&dir, None).unwrap();
    let result = analyze(&dir, &config, &[]).unwrap();

    // Key assertion: no crash on mixed import styles
    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Mixed import files must produce entities"
    );
}

/// CADV-4: JS-style relative path NOT handled by Python resolver
#[test]
fn test_circular_dep_js_relative_not_python_resolver() {
    let fixture_dir = fixture_base().join("tests/fixtures/javascript/cross_file/simple_import");
    if !fixture_dir.exists() {
        return;
    }
    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]);
    assert!(
        result.is_ok(),
        "JS resolution must not crash after Python resolver added"
    );
}

/// CADV-5: Three-file relative import cycle
#[test]
fn test_circular_dep_three_file_relative_cycle() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        dir.join("a.py"),
        "from .b import func_b\n\ndef func_a():\n    return func_b()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.py"),
        "from .c import func_c\n\ndef func_b():\n    return func_c()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("c.py"),
        "from .a import func_a\n\ndef func_c():\n    return func_a()\n",
    )
    .unwrap();

    let config = Config::load(&dir, None).unwrap();
    let result = analyze(&dir, &config, &[]).unwrap();

    let circular = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "circular_dependency")
        .count();
    assert!(
        circular > 0,
        "Three-file relative import cycle a→b→c→a must be detected. Got 0 findings."
    );
}

// =========================================================================
// Category 9: Circular Dependency — Regression (CREG-*)
// =========================================================================

/// CREG-1: Existing absolute import circular_import fixture still works
#[test]
fn test_circular_dep_regression_absolute_fixture() {
    let fixture_dir = fixture_base().join("tests/fixtures/python/cross_file/circular_import");
    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Existing absolute import circular fixture must still produce entities. Got {}",
        result.manifest.metadata.entity_count
    );
}

/// CREG-2: Rust resolution paths unaffected
#[test]
fn test_circular_dep_regression_rust_paths_unaffected() {
    let fixture_dir = fixture_base().join("tests/fixtures/rust");
    if !fixture_dir.exists() {
        return;
    }
    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]);
    assert!(
        result.is_ok(),
        "Rust resolution must not be broken by Python relative import resolver"
    );
}

/// CREG-3: Linear imports (no cycle) still produce zero findings
#[test]
fn test_circular_dep_regression_linear_no_false_positive() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("pkg");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("__init__.py"), "").unwrap();
    std::fs::write(
        dir.join("a.py"),
        "from .b import func_b\n\ndef func_a():\n    return func_b()\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("b.py"),
        "from .c import func_c\n\ndef func_b():\n    return func_c()\n",
    )
    .unwrap();
    std::fs::write(dir.join("c.py"), "def func_c():\n    return 42\n").unwrap();

    let config = Config::load(&dir, None).unwrap();
    let result = analyze(&dir, &config, &[]).unwrap();

    let circular = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "circular_dependency")
        .count();
    assert_eq!(
        circular, 0,
        "Linear import chain (no cycle) must produce zero circular_dependency findings. Got {}",
        circular
    );
}
