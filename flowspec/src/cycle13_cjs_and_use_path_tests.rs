//! QA-Foundation (QA 1) — Cycle 13: JS CJS destructured require + Rust `use` qualified path tests.
//!
//! 24 tests across 7 categories validating:
//! - JS CJS destructured require cross-file resolution (D1–D7)
//! - Rust `use` qualified path phantom dependency fix (U1–U8)
//! - Adversarial edge cases (A1–A4)
//! - Rust cross-file fixtures (F1–F3)
//! - Integration cross-pattern interaction (I1–I2)

use std::path::PathBuf;

use crate::config::Config;
use crate::graph::{populate_graph, resolve_cross_file_imports, Graph};
use crate::parser::ir::{EdgeKind, Symbol, SymbolId, SymbolKind};
use crate::parser::javascript::JsAdapter;
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;

/// Parse multiple JS source files and run cross-file resolution.
fn cross_file_graph_js(files: &[(&str, &str)]) -> Graph {
    let adapter = JsAdapter;
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

/// Parse multiple Rust source files and run cross-file resolution.
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

/// Check if a symbol has at least one incoming References or Calls edge from a same-file symbol.
fn has_same_file_reference(graph: &Graph, target_id: SymbolId) -> bool {
    let target_sym = graph.get_symbol(target_id).unwrap();
    let target_file = &target_sym.location.file;
    graph.edges_to(target_id).iter().any(|edge| {
        if let Some(source_sym) = graph.get_symbol(edge.target) {
            source_sym.location.file == *target_file
                && edge.target != target_id
                && matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)
        } else {
            false
        }
    })
}

// =========================================================================
// Category 1: JS CJS Destructured Require — Basic (D1–D3)
// =========================================================================

/// D1: Single destructured binding resolves cross-file.
#[test]
fn test_js_cjs_destructured_single_binding_resolves() {
    let graph = cross_file_graph_js(&[
        (
            "utils.js",
            "function process(data) { return data; }\nmodule.exports = { process };",
        ),
        (
            "app.js",
            "const { process } = require('./utils');\nfunction run() { process(\"test\"); }",
        ),
    ]);

    // Import symbol named "process" must exist in app.js
    let import = find_import_symbol(&graph, "process");
    assert!(
        import.is_some(),
        "Import symbol 'process' must exist for destructured CJS require"
    );

    let (_, sym) = import.unwrap();
    assert!(
        sym.annotations.contains(&"cjs".to_string()),
        "CJS destructured import must have 'cjs' annotation, got {:?}",
        sym.annotations
    );
    assert!(
        sym.annotations.contains(&"from:./utils".to_string()),
        "Import must have from:./utils annotation, got {:?}",
        sym.annotations
    );
    assert_eq!(
        sym.kind,
        SymbolKind::Variable,
        "Destructured CJS binding must be Variable, not Module"
    );

    // Cross-file resolution: the definition in utils.js must have callers
    let def = find_definition(&graph, "process");
    assert!(def.is_some(), "process definition must exist in utils.js");
    let (def_id, _) = def.unwrap();
    let callers = graph.callers(def_id);
    assert!(
        !callers.is_empty(),
        "process in utils.js must have cross-file callers from app.js"
    );
}

/// D2: Multiple destructured bindings each resolve independently.
#[test]
fn test_js_cjs_destructured_multiple_bindings_resolve() {
    let graph = cross_file_graph_js(&[
        (
            "utils.js",
            "function foo() {}\nfunction bar() {}\nmodule.exports = { foo, bar };",
        ),
        (
            "app.js",
            "const { foo, bar } = require('./utils');\nfunction run() { foo(); bar(); }",
        ),
    ]);

    // TWO import symbols must exist
    let foo_import = find_import_symbol(&graph, "foo");
    let bar_import = find_import_symbol(&graph, "bar");
    assert!(foo_import.is_some(), "Import 'foo' must exist");
    assert!(bar_import.is_some(), "Import 'bar' must exist");

    // No symbol named "{ foo, bar }" should exist (regression guard)
    let collapsed = graph
        .all_symbols()
        .any(|(_, s)| s.name.contains('{') || s.name.contains('}'));
    assert!(
        !collapsed,
        "No symbol with braces in name should exist — each binding must be separate"
    );

    // Both must resolve cross-file
    let foo_def = find_definition(&graph, "foo");
    let bar_def = find_definition(&graph, "bar");
    assert!(foo_def.is_some(), "foo definition must exist in utils.js");
    assert!(bar_def.is_some(), "bar definition must exist in utils.js");

    let (foo_def_id, _) = foo_def.unwrap();
    let (bar_def_id, _) = bar_def.unwrap();
    assert!(
        !graph.callers(foo_def_id).is_empty(),
        "foo must have cross-file callers"
    );
    assert!(
        !graph.callers(bar_def_id).is_empty(),
        "bar must have cross-file callers"
    );
}

/// D3: Aliased destructured binding resolves via original_name.
#[test]
fn test_js_cjs_destructured_alias_resolves() {
    let graph = cross_file_graph_js(&[
        (
            "utils.js",
            "function process(data) { return data; }\nmodule.exports = { process };",
        ),
        (
            "app.js",
            "const { process: handler } = require('./utils');\nfunction run() { handler(\"test\"); }",
        ),
    ]);

    // Import symbol named "handler" (the local alias) must exist
    let import = find_import_symbol(&graph, "handler");
    assert!(
        import.is_some(),
        "Import symbol 'handler' (alias) must exist"
    );

    let (_, sym) = import.unwrap();
    assert!(
        sym.annotations
            .contains(&"original_name:process".to_string()),
        "Aliased import must have original_name:process annotation, got {:?}",
        sym.annotations
    );

    // Cross-file resolution uses "process" (original_name) to look up in utils.js
    let def = find_definition(&graph, "process");
    assert!(def.is_some(), "process definition must exist in utils.js");
    let (def_id, _) = def.unwrap();
    let callers = graph.callers(def_id);
    assert!(
        !callers.is_empty(),
        "process in utils.js must have cross-file callers via aliased import"
    );
}

// =========================================================================
// Category 2: JS CJS Destructured Require — Mixed & Integration (D4–D7)
// =========================================================================

/// D4: Mixed CJS/ESM in same project both resolve.
#[test]
fn test_js_mixed_esm_and_cjs_destructured_resolve() {
    let graph = cross_file_graph_js(&[
        (
            "utils.js",
            "function helper() {}\nmodule.exports = { helper };",
        ),
        (
            "math.js",
            "export function add(a, b) { return a + b; }",
        ),
        (
            "app.js",
            "const { helper } = require('./utils');\nimport { add } from './math';\nfunction run() { helper(); add(1,2); }",
        ),
    ]);

    // Both imports must exist
    let helper_import = find_import_symbol(&graph, "helper");
    let add_import = find_import_symbol(&graph, "add");
    assert!(
        helper_import.is_some(),
        "CJS destructured import 'helper' must exist"
    );
    assert!(add_import.is_some(), "ESM import 'add' must exist");

    // CJS import has "cjs" annotation, ESM does not
    let (_, helper_sym) = helper_import.unwrap();
    assert!(
        helper_sym.annotations.contains(&"cjs".to_string()),
        "CJS import must have 'cjs' annotation"
    );
    let (_, add_sym) = add_import.unwrap();
    assert!(
        !add_sym.annotations.contains(&"cjs".to_string()),
        "ESM import must NOT have 'cjs' annotation"
    );
}

/// D5: Destructured + whole-module require in same file.
#[test]
fn test_js_cjs_destructured_and_whole_module_coexist() {
    let graph = cross_file_graph_js(&[
        (
            "utils.js",
            "function process() {}\nmodule.exports = { process };",
        ),
        (
            "logger.js",
            "function log() {}\nmodule.exports = { log };",
        ),
        (
            "app.js",
            "const { process } = require('./utils');\nconst logger = require('./logger');\nfunction run() { process(); logger.log(); }",
        ),
    ]);

    // "process" is Variable (destructured), "logger" is Module (whole-module)
    let process_import = find_import_symbol(&graph, "process");
    let logger_import = find_import_symbol(&graph, "logger");
    assert!(
        process_import.is_some(),
        "Destructured import 'process' must exist"
    );
    assert!(
        logger_import.is_some(),
        "Whole-module import 'logger' must exist"
    );

    let (_, p_sym) = process_import.unwrap();
    assert_eq!(
        p_sym.kind,
        SymbolKind::Variable,
        "Destructured import must be Variable"
    );

    let (_, l_sym) = logger_import.unwrap();
    assert_eq!(
        l_sym.kind,
        SymbolKind::Module,
        "Whole-module import must be Module"
    );
}

/// D6: Empty destructuring does not crash.
#[test]
fn test_js_cjs_empty_destructuring_no_crash() {
    let graph = cross_file_graph_js(&[
        ("utils.js", "module.exports = {};"),
        (
            "app.js",
            "const {} = require('./utils');\nfunction run() {}",
        ),
    ]);

    // No crash is the primary assertion. Also verify no garbage import symbols.
    let brace_symbols: Vec<_> = graph
        .all_symbols()
        .filter(|(_, s)| s.name.contains('{') || s.name.contains('}'))
        .collect();
    assert!(
        brace_symbols.is_empty(),
        "Empty destructuring must not produce brace-named symbols"
    );
}

/// D7: Nested destructuring does not crash (adversarial).
#[test]
fn test_js_cjs_nested_destructuring_no_crash() {
    let graph = cross_file_graph_js(&[
        ("utils.js", "module.exports = { x: { y: 1 } };"),
        (
            "app.js",
            "const { x: { y } } = require('./utils');\nfunction run() { console.log(y); }",
        ),
    ]);

    // No crash is the primary assertion
    let _count = graph.all_symbols().count();
}

// =========================================================================
// Category 3: Rust `use` Qualified Path — Core (U1–U4)
// =========================================================================

/// U1: use + qualified call creates import usage reference.
#[test]
fn test_rust_use_qualified_call_not_phantom() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::fs;\nfn read_config() {\n    let _ = fs::read_to_string(\"config.toml\");\n}",
    )]);

    // Import symbol "fs" must exist
    let import = find_import_symbol(&graph, "fs");
    assert!(import.is_some(), "Import symbol 'fs' must exist");

    let (import_id, sym) = import.unwrap();
    assert!(
        sym.annotations.iter().any(|a| a.starts_with("from:")),
        "fs import must have from: annotation"
    );

    // A reference edge must exist FROM read_config TO the "fs" import symbol
    assert!(
        has_same_file_reference(&graph, import_id),
        "fs import must have a same-file reference from read_config — Issue #15 fix"
    );
}

/// U2: Crate-internal use + qualified call resolves cross-file.
#[test]
fn test_rust_crate_use_qualified_call_resolves() {
    let graph = cross_file_graph_rust(&[
        (
            "src/parser.rs",
            "pub fn parse() -> String { String::new() }",
        ),
        (
            "src/main.rs",
            "use crate::parser;\nfn run() { parser::parse(); }",
        ),
    ]);

    // Import symbol "parser" in main.rs must have a same-file reference
    let import = find_import_symbol(&graph, "parser");
    assert!(import.is_some(), "Import 'parser' must exist in main.rs");
    let (import_id, _) = import.unwrap();
    assert!(
        has_same_file_reference(&graph, import_id),
        "parser import must have same-file reference from run()"
    );
}

/// U3: Multi-segment qualified call preserves prefix reference.
#[test]
fn test_rust_multi_segment_qualified_call() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::collections;\nfn build() {\n    let _map = collections::HashMap::new();\n}",
    )]);

    let import = find_import_symbol(&graph, "collections");
    assert!(import.is_some(), "Import 'collections' must exist");
    let (import_id, _) = import.unwrap();
    assert!(
        has_same_file_reference(&graph, import_id),
        "collections import must have same-file reference from build()"
    );
}

/// U4: Use-with-alias + qualified call tracks alias.
#[test]
fn test_rust_use_alias_qualified_call() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::fs as filesystem;\nfn read_it() {\n    let _ = filesystem::read_to_string(\"file.txt\");\n}",
    )]);

    let import = find_import_symbol(&graph, "filesystem");
    assert!(import.is_some(), "Import 'filesystem' (alias) must exist");
    let (import_id, _) = import.unwrap();
    assert!(
        has_same_file_reference(&graph, import_id),
        "filesystem import must have same-file reference from read_it()"
    );
}

// =========================================================================
// Category 4: Rust `use` Qualified Path — Regression Guards (U5–U8)
// =========================================================================

/// U5: Direct function calls still resolve (regression guard).
#[test]
fn test_rust_direct_call_still_resolves() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "fn helper() -> i32 { 42 }\nfn main() { let _x = helper(); }",
    )]);

    let def = find_definition(&graph, "helper");
    assert!(def.is_some(), "helper definition must exist");
    let (def_id, _) = def.unwrap();
    let callers = graph.callers(def_id);
    assert!(
        !callers.is_empty(),
        "Direct call from main to helper must still resolve"
    );
}

/// U6: self.method() calls still resolve (regression guard).
#[test]
fn test_rust_self_method_still_resolves() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "struct Foo;\nimpl Foo {\n    fn bar(&self) {}\n    fn baz(&self) { self.bar(); }\n}",
    )]);

    let bar_def = find_definition(&graph, "bar");
    assert!(bar_def.is_some(), "bar method must exist");
    let (bar_id, _) = bar_def.unwrap();
    let callers = graph.callers(bar_id);
    assert!(
        !callers.is_empty(),
        "self.bar() call from baz must still resolve"
    );
}

/// U7: Actually unused import STILL flagged as phantom.
#[test]
fn test_rust_truly_unused_import_still_phantom() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::io;\nfn main() {\n    println!(\"no io usage\");\n}",
    )]);

    let import = find_import_symbol(&graph, "io");
    assert!(import.is_some(), "Import 'io' must exist");
    let (import_id, _) = import.unwrap();

    // io must NOT have any same-file references since it's never used in a qualified call
    assert!(
        !has_same_file_reference(&graph, import_id),
        "Genuinely unused import 'io' must have zero same-file references"
    );
}

/// U8: Scoped identifier without import prefix resolves normally.
#[test]
fn test_rust_scoped_without_import_prefix() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "struct MyMap;\nimpl MyMap {\n    fn new() -> Self { MyMap }\n}\nfn build() { let _m = MyMap::new(); }",
    )]);

    // The call MyMap::new() should try to resolve. Check no crash.
    let _count = graph.all_symbols().count();
}

// =========================================================================
// Category 5: Adversarial & Edge Cases (A1–A4)
// =========================================================================

/// A1: Shadowing — local function same name as import prefix.
#[test]
fn test_rust_shadow_local_vs_import_prefix() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::fs;\nfn fs() -> i32 { 42 }\nfn main() {\n    fs::read_to_string(\"x\");\n    fs();\n}",
    )]);

    // Both the import "fs" and function "fs" should exist
    let import = find_import_symbol(&graph, "fs");
    let func = find_definition(&graph, "fs");
    assert!(import.is_some(), "Import 'fs' must exist");
    assert!(func.is_some(), "Function 'fs' must exist");

    // Both should have references (neither phantom)
    let (import_id, _) = import.unwrap();
    let (func_id, _) = func.unwrap();
    // At minimum, the import should have a reference from the qualified call
    assert!(
        has_same_file_reference(&graph, import_id),
        "Import 'fs' should have a reference from qualified call"
    );
    // The function should have a caller from the direct fs() call
    assert!(
        !graph.callers(func_id).is_empty(),
        "Function 'fs' should have a caller from the direct call"
    );
}

/// A2: Deeply nested scoped path.
#[test]
fn test_rust_deep_scoped_path() {
    let graph = cross_file_graph_rust(&[(
        "src/lib.rs",
        "use std::collections::hash_map;\nfn build() {\n    let _ = hash_map::HashMap::new();\n}",
    )]);

    let import = find_import_symbol(&graph, "hash_map");
    assert!(import.is_some(), "Import 'hash_map' must exist");
    let (import_id, _) = import.unwrap();
    assert!(
        has_same_file_reference(&graph, import_id),
        "hash_map import must have same-file reference from build()"
    );
}

/// A3: CJS require with computed property (adversarial).
#[test]
fn test_js_cjs_computed_destructure_no_crash() {
    // Computed property names produce different AST nodes
    let graph = cross_file_graph_js(&[
        ("utils.js", "module.exports = { process: function() {} };"),
        (
            "app.js",
            "const key = 'process';\nconst { [key]: fn } = require('./utils');",
        ),
    ]);

    // No crash is the primary assertion
    let _count = graph.all_symbols().count();
}

/// A4: CJS destructured require from non-relative path.
#[test]
fn test_js_cjs_destructured_external_module_no_crash() {
    let graph = cross_file_graph_js(&[(
        "app.js",
        "const { readFileSync } = require('fs');\nfunction read() { readFileSync('x'); }",
    )]);

    // Import symbol must be created even for external modules
    let import = find_import_symbol(&graph, "readFileSync");
    assert!(
        import.is_some(),
        "Import 'readFileSync' must be created for external CJS destructured require"
    );
    let (_, sym) = import.unwrap();
    assert!(
        sym.annotations.contains(&"from:fs".to_string()),
        "External CJS import must have from:fs annotation"
    );
}

// =========================================================================
// Category 6: Rust Fixture Validation (F1–F3)
// =========================================================================

/// F1 (REPLACEMENT): Rust cross-file fixture with documented properties.
#[test]
fn test_rust_cross_file_fixture_known_properties() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/rust/cross_file");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = crate::analyze(&fixture_dir, &config, &[]).unwrap();

    // At least 4 functions: entry_point, handle, helper, unused_helper
    let function_entities: Vec<_> = result
        .manifest
        .entities
        .iter()
        .filter(|e| e.kind == "fn")
        .collect();
    assert!(
        function_entities.len() >= 4,
        "Fixture must produce at least 4 function entities, got {}",
        function_entities.len()
    );

    // helper must exist as an entity
    let helper_entity =
        result.manifest.entities.iter().find(|e| {
            e.id.contains("helper") && !e.id.contains("unused") && !e.id.contains("import")
        });
    assert!(
        helper_entity.is_some(),
        "helper function must appear in manifest"
    );

    // unused_helper must exist as an entity with 0 callers
    let unused_entity = result
        .manifest
        .entities
        .iter()
        .find(|e| e.id.contains("unused_helper") && !e.id.contains("import"));
    assert!(
        unused_entity.is_some(),
        "unused_helper function must appear in manifest"
    );
    if let Some(entity) = unused_entity {
        assert!(
            entity.called_by.is_empty(),
            "unused_helper must have 0 callers, got {:?}",
            entity.called_by
        );
    }

    // entry_point and handle must exist
    assert!(
        result
            .manifest
            .entities
            .iter()
            .any(|e| e.id.contains("entry_point")),
        "entry_point must appear in manifest"
    );
    assert!(
        result
            .manifest
            .entities
            .iter()
            .any(|e| e.id.ends_with("handle") || e.id.contains("::handle")),
        "handle function must appear in manifest"
    );
}

/// F2: Rust fixture module map correctness.
#[test]
fn test_rust_cross_file_fixture_module_map() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/rust/cross_file");

    let paths: Vec<PathBuf> = ["lib.rs", "utils.rs", "handler.rs"]
        .iter()
        .map(|f| fixture_dir.join(f))
        .collect();

    let module_map = crate::build_module_map(&paths);

    // Module map must contain keys for utils and handler
    let has_utils = module_map.keys().any(|k| k.contains("utils"));
    let has_handler = module_map.keys().any(|k| k.contains("handler"));
    assert!(has_utils, "Module map must contain a key for utils");
    assert!(has_handler, "Module map must contain a key for handler");
}

/// F3: Rust fixture phantom_dependency accuracy.
#[test]
fn test_rust_cross_file_fixture_phantom_accuracy() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/rust/cross_file");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = crate::analyze(&fixture_dir, &config, &[]).unwrap();

    // Check diagnostics for phantom_dependency false positives on this fixture
    let phantom_findings: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "phantom_dependency")
        .collect();

    // The "utils" import in handler.rs should NOT be phantom (qualified call usage via U1 fix)
    let utils_phantom = phantom_findings.iter().any(|d| d.entity.contains("utils"));
    assert!(
        !utils_phantom,
        "utils import in handler.rs must NOT be flagged as phantom (Issue #15 fix)"
    );
}

// =========================================================================
// Category 7: Integration — Cross-Pattern Interaction (I1–I2)
// =========================================================================

/// I1: CJS destructured import does NOT trigger stale_reference.
#[test]
fn test_js_cjs_destructured_not_stale_reference() {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("tests/fixtures/javascript/cross_file/cjs_destructured");

    let config = Config::load(&fixture_dir, None).unwrap();
    let result = crate::analyze(&fixture_dir, &config, &[]).unwrap();

    let stale_findings: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "stale_reference")
        .filter(|d| d.entity == "process" || d.entity == "helper")
        .collect();
    assert!(
        stale_findings.is_empty(),
        "CJS destructured imports must not trigger stale_reference: {:?}",
        stale_findings.iter().map(|d| &d.entity).collect::<Vec<_>>()
    );
}

/// I2: Rust use-path fix does not suppress phantom for actual unused.
#[test]
fn test_rust_use_path_fix_preserves_phantom_for_unused() {
    let graph = cross_file_graph_rust(&[("src/lib.rs", "use std::io;\nfn main() {}")]);

    let import = find_import_symbol(&graph, "io");
    assert!(import.is_some(), "Import 'io' must exist");
    let (import_id, _) = import.unwrap();
    assert!(
        !has_same_file_reference(&graph, import_id),
        "Unused import 'io' must NOT have same-file references — phantom must still fire"
    );
}
