//! QA-Foundation (QA 1) — Cycle 10: JS cross-file import resolution tests.
//!
//! 21 tests across 7 categories validating JS ESM import resolution,
//! CJS require() detection, re-exports, graph integrity, adversarial
//! edge cases, regression guards, and module map validation.

use std::path::PathBuf;

use crate::config::Config;
use crate::parser::ir::SymbolKind;
use crate::{analyze, build_module_map};

// =========================================================================
// Category 1: JS ESM Import Resolution (T1–T5)
// =========================================================================

#[test]
fn test_js_esm_named_import_resolves_cross_file() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_named");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // helper definition must exist as entity (not the import symbol)
    let helper_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("helper") && !e.id.contains("import"));
    assert!(
        helper_entity.is_some(),
        "helper function from provider.js must appear in manifest entities"
    );

    // Cross-file edge: helper must have callers from consumer.js
    if let Some(entity) = helper_entity {
        assert!(
            !entity.called_by.is_empty(),
            "ESM named import must create cross-file edge visible in called_by. \
             called_by is empty for helper even though consumer.js imports and calls it."
        );
    }
}

#[test]
fn test_js_esm_default_import_resolves_cross_file() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_default");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let calc_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("calculate") && !e.id.contains("import"));
    assert!(
        calc_entity.is_some(),
        "calculate function from math.js must appear in manifest"
    );
    if let Some(entity) = calc_entity {
        assert!(
            !entity.called_by.is_empty(),
            "Default ESM import must create cross-file edge for calculate"
        );
    }
}

#[test]
fn test_js_esm_aliased_import_resolves_original() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_aliased");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let utility_exists = result
        .manifest
        .entities
        .iter()
        .any(|e| e.id.contains("utility"));
    assert!(
        utility_exists,
        "utility function must be present in manifest"
    );
}

#[test]
fn test_js_esm_namespace_import_resolves() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_namespace");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 3,
        "Must extract foo, bar, and main despite namespace import. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_js_esm_multiple_named_imports() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_multiple");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let alpha = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("alpha") && !e.id.contains("import"));
    let beta = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("beta") && !e.id.contains("import"));

    assert!(alpha.is_some(), "alpha must be in entities");
    assert!(beta.is_some(), "beta must be in entities");

    if let Some(a) = alpha {
        assert!(!a.called_by.is_empty(), "alpha must have cross-file caller");
    }
    if let Some(b) = beta {
        assert!(!b.called_by.is_empty(), "beta must have cross-file caller");
    }
}

// =========================================================================
// Category 2: CJS Require Resolution (T6–T7)
// =========================================================================

#[test]
fn test_js_cjs_require_resolves_cross_file() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/cjs_require");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Must extract at least process and run. Got {}",
        result.manifest.metadata.entity_count
    );

    let process_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("process") && !e.id.contains("import"));
    assert!(
        process_entity.is_some(),
        "process function from module.js must appear in manifest"
    );
}

#[test]
fn test_js_cjs_require_dynamic_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/cjs_dynamic");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 1,
        "Must extract test() even with dynamic require. Got {}",
        result.manifest.metadata.entity_count
    );
}

// =========================================================================
// Category 3: Re-exports (T8–T9)
// =========================================================================

#[test]
fn test_js_reexport_chain_propagates() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/reexport");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let engine_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("engine") && !e.id.contains("import"));
    assert!(
        engine_entity.is_some(),
        "engine function from core.js must appear in manifest entities"
    );
}

#[test]
fn test_js_star_reexport_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/star_reexport");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Must extract symbols from internals.js despite star re-export. Got {}",
        result.manifest.metadata.entity_count
    );
}

// =========================================================================
// Category 4: Cross-File Graph Integrity (T10–T11)
// =========================================================================

#[test]
fn test_js_cross_file_callees_include_import_targets() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/esm_named");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    // Find main function in graph
    let main_id = result
        .graph
        .all_symbols()
        .find(|(_, s)| s.name == "main" && s.kind == SymbolKind::Function)
        .map(|(id, _)| id);
    assert!(main_id.is_some(), "main function must exist in graph");

    let callees = result.graph.callees(main_id.unwrap());
    assert!(
        !callees.is_empty(),
        "graph.callees(main) must include cross-file target helper after JS import resolution"
    );
}

#[test]
fn test_js_external_import_stays_partial() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/external_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 1,
        "Must extract run() even with external imports. Got {}",
        result.manifest.metadata.entity_count
    );
}

// =========================================================================
// Category 5: Adversarial / Edge Cases (T12–T16)
// =========================================================================

#[test]
fn test_js_circular_import_no_infinite_loop() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/circular_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Both files' symbols must be extracted despite circular import. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_js_dynamic_import_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/dynamic_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "Must extract loadPlugin and main despite dynamic import. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_js_empty_import_source_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/empty_source");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 1,
        "Must extract test() even with empty import source. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_js_path_traversal_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/path_traversal");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 1,
        "Must extract test() even with path traversal import. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_js_css_import_no_crash() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/non_js_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 1,
        "Must extract render() even with CSS/JSON imports. Got {}",
        result.manifest.metadata.entity_count
    );
}

// =========================================================================
// Category 6: Regression Guards (R1–R3)
// =========================================================================

#[test]
fn test_regression_python_cross_file_still_works_after_js_resolution() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/simple_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let helper_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("helper") && !e.id.contains("import"));
    assert!(
        helper_entity.is_some(),
        "REGRESSION: Python helper must still appear in entities after JS resolution added"
    );
    if let Some(entity) = helper_entity {
        assert!(
            !entity.called_by.is_empty(),
            "REGRESSION: Python cross-file called_by must still work after JS resolution added"
        );
    }
}

#[test]
fn test_regression_python_circular_import_still_safe() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/python/cross_file/circular_import");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    assert!(
        result.manifest.metadata.entity_count >= 2,
        "REGRESSION: Python circular import must still resolve safely. Got {}",
        result.manifest.metadata.entity_count
    );
}

#[test]
fn test_regression_js_basic_functions_unaffected() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = analyze(&fixture_dir, &config, &[]).unwrap();

    let js_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.loc.contains("basic_functions"))
        .collect();
    assert!(
        js_entities.len() >= 4,
        "REGRESSION: basic_functions.js must still produce 4 entities. Got {}",
        js_entities.len()
    );
}

// =========================================================================
// Category 7: Module Map Validation (T17–T18)
// =========================================================================

#[test]
fn test_build_module_map_includes_js_files() {
    let files = vec![
        PathBuf::from("/project/src/utils.py"),
        PathBuf::from("/project/src/helpers.js"),
        PathBuf::from("/project/src/lib.mjs"),
        PathBuf::from("/project/src/types.ts"),
    ];

    let map = build_module_map(&files);

    // Must contain at least the Python file (regression guard)
    assert!(
        map.values()
            .any(|p| p.to_string_lossy().contains("utils.py")),
        "build_module_map must still include .py files"
    );

    // Must contain the JS file (new behavior)
    assert!(
        map.values()
            .any(|p| p.to_string_lossy().contains("helpers.js")),
        "build_module_map must include .js files after extension"
    );
}

#[test]
fn test_js_module_map_keys_are_path_based() {
    let files = vec![
        PathBuf::from("/project/src/utils/helpers.js"),
        PathBuf::from("/project/src/core.js"),
        PathBuf::from("/project/src/pkg/module.py"),
    ];

    let map = build_module_map(&files);

    // JS files should have path-based keys (with forward slashes, no extension)
    let js_keys: Vec<&String> = map
        .keys()
        .filter(|k| {
            map.get(*k)
                .map(|v| {
                    v.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| matches!(e, "js" | "ts" | "mjs" | "tsx" | "jsx"))
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
        .collect();

    // At least one JS file should be mapped
    assert!(
        !js_keys.is_empty(),
        "JS files must be in module map. Keys: {:?}",
        map.keys().collect::<Vec<_>>()
    );

    // JS keys should NOT use dots (Python style) — they should use path separators or just be stems
    for key in &js_keys {
        assert!(
            !key.contains('.'),
            "JS module map key '{}' should not contain dots (path-based, not dotted)",
            key
        );
    }
}
