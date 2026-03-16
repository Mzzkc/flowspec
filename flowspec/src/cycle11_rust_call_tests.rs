// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Cycle 11 QA-Foundation tests: Rust intra-file call resolution + .cjs extension fix.
//!
//! Tests verify that the Rust adapter produces call edges in the graph via
//! `extract_call()` → `"call:<name>"` → `populate_graph()` → `resolve_callee()`.

use std::path::PathBuf;

use crate::config::Config;
use crate::graph::{populate_graph, Graph};
use crate::parser::ir::{Symbol, SymbolId};
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse Rust source and populate a graph. Returns the graph and temp file path.
fn parse_and_populate_rust(source: &str) -> (Graph, PathBuf) {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("test_module.rs");
    std::fs::write(&path, source).expect("write fixture");
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(&path, source).expect("parse");
    let mut graph = Graph::new();
    populate_graph(&mut graph, &result);
    std::mem::forget(dir);
    (graph, path)
}

/// Find a symbol by name in the graph. Panics if not found.
fn find_symbol<'a>(graph: &'a Graph, name: &str) -> (SymbolId, &'a Symbol) {
    graph
        .all_symbols()
        .find(|(_, s)| s.name == name)
        .unwrap_or_else(|| panic!("Symbol '{}' not found in graph", name))
}

// ===========================================================================
// Category 1: Direct Function Call Resolution (T1–T2)
// ===========================================================================

#[test]
fn test_rust_direct_function_call_creates_edge() {
    let source = r#"
fn helper() -> i32 {
    42
}

fn main() {
    helper();
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (main_id, _) = find_symbol(&graph, "main");
    let (helper_id, _) = find_symbol(&graph, "helper");

    let callees = graph.callees(main_id);
    assert!(
        callees.contains(&helper_id),
        "main()'s callees must include helper. Direct function call resolution broken."
    );

    let callers = graph.callers(helper_id);
    assert!(
        callers.contains(&main_id),
        "helper's callers must include main. Bidirectional edge insertion broken."
    );
}

#[test]
fn test_rust_multiple_calls_from_one_function() {
    let source = r#"
fn foo() -> i32 { 1 }
fn bar() -> i32 { 2 }
fn baz() -> i32 { 3 }

fn caller() {
    foo();
    bar();
    baz();
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (caller_id, _) = find_symbol(&graph, "caller");
    let (foo_id, _) = find_symbol(&graph, "foo");
    let (bar_id, _) = find_symbol(&graph, "bar");
    let (baz_id, _) = find_symbol(&graph, "baz");

    let callees = graph.callees(caller_id);
    assert!(callees.contains(&foo_id), "callees must include foo");
    assert!(callees.contains(&bar_id), "callees must include bar");
    assert!(callees.contains(&baz_id), "callees must include baz");
    assert_eq!(
        callees.len(),
        3,
        "Exactly 3 callees expected, got {}",
        callees.len()
    );
}

// ===========================================================================
// Category 2: Self-Method Call Resolution (T3–T4)
// ===========================================================================

#[test]
fn test_rust_self_method_call_resolves() {
    let source = r#"
struct Foo {
    x: i32,
}

impl Foo {
    fn helper(&self) -> i32 {
        self.x
    }

    fn run(&self) {
        self.helper();
    }
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (run_id, _) = find_symbol(&graph, "run");
    let (helper_id, _) = find_symbol(&graph, "helper");

    let callees = graph.callees(run_id);
    assert!(
        callees.contains(&helper_id),
        "self.helper() must resolve to helper method. \
         Expected run's callees to contain helper_id. Got: {:?}",
        callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_rust_non_self_method_call_stays_unresolved() {
    let source = r#"
struct Foo;

impl Foo {
    fn do_thing(&self) {}
}

fn caller() {
    let f = Foo;
    f.do_thing();
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (caller_id, _) = find_symbol(&graph, "caller");

    let callee_names: Vec<String> = graph
        .callees(caller_id)
        .iter()
        .filter_map(|id| graph.get_symbol(*id))
        .map(|s| s.name.clone())
        .collect();

    assert!(
        !callee_names.contains(&"do_thing".to_string()),
        "f.do_thing() requires type inference — must NOT resolve intra-file. \
         Resolved callees: {:?}",
        callee_names
    );
}

// ===========================================================================
// Category 3: Associated Function / Qualified Path Resolution (T5)
// ===========================================================================

#[test]
fn test_rust_associated_function_resolves() {
    let source = r#"
struct Foo;

impl Foo {
    fn new() -> Self {
        Foo
    }
}

fn main() {
    let _f = Foo::new();
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (main_id, _) = find_symbol(&graph, "main");
    let (new_id, _) = find_symbol(&graph, "new");

    let callees = graph.callees(main_id);
    assert!(
        callees.contains(&new_id),
        "Foo::new() must resolve to the new() associated function. \
         Callees: {:?}",
        callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
}

// ===========================================================================
// Category 4: Adversarial / Edge Cases (T6–T9)
// ===========================================================================

#[test]
fn test_rust_recursive_call_creates_self_edge() {
    let source = r#"
fn factorial(n: u64) -> u64 {
    if n <= 1 { 1 } else { n * factorial(n - 1) }
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (factorial_id, _) = find_symbol(&graph, "factorial");

    let callees = graph.callees(factorial_id);
    assert!(
        callees.contains(&factorial_id),
        "Recursive call must create self-edge. factorial's callees must include itself."
    );
}

#[test]
fn test_rust_call_to_undefined_function_no_crash() {
    let source = r#"
fn main() {
    nonexistent_function();
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (main_id, _) = find_symbol(&graph, "main");

    let callees = graph.callees(main_id);
    assert!(
        callees.is_empty(),
        "Call to undefined function must produce no edge. Got callees: {:?}",
        callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_rust_macro_invocations_not_treated_as_calls() {
    let source = r#"
fn main() {
    println!("hello");
    vec![1, 2, 3];
    assert_eq!(1, 1);
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (main_id, _) = find_symbol(&graph, "main");

    let callees = graph.callees(main_id);
    assert!(
        callees.is_empty(),
        "Macro invocations (println!, vec!, assert_eq!) must NOT create call edges. \
         Got {} callees: {:?}",
        callees.len(),
        callees
            .iter()
            .filter_map(|id| graph.get_symbol(*id))
            .map(|s| &s.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn test_rust_chained_method_calls_no_crash() {
    let source = r#"
fn process(data: Vec<i32>) -> Vec<i32> {
    data.iter().map(|x| x + 1).collect()
}
"#;
    let (graph, _) = parse_and_populate_rust(source);
    let (process_id, _) = find_symbol(&graph, "process");

    // Should not crash — this is the primary assertion
    let callees = graph.callees(process_id);

    let callee_names: Vec<String> = callees
        .iter()
        .filter_map(|id| graph.get_symbol(*id))
        .map(|s| s.name.clone())
        .collect();

    // iter/map/collect are stdlib trait methods — no intra-file resolution target
    assert!(
        !callee_names.contains(&"iter".to_string()),
        "iter() is a stdlib method, must not resolve intra-file"
    );
    assert!(
        !callee_names.contains(&"collect".to_string()),
        "collect() is a stdlib method, must not resolve intra-file"
    );
}

// ===========================================================================
// Category 5: .cjs Extension Fix (C1–C2)
// ===========================================================================

#[test]
fn test_cjs_files_included_in_analysis() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(
        tmp.path().join("utils.cjs"),
        "function helper() { return 42; }\nmodule.exports = { helper };\n",
    )
    .unwrap();

    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    assert!(
        !result.manifest.entities.is_empty(),
        ".cjs file must be discovered and parsed. \
         No entities found — check discover_source_files() match arm, \
         JS_EXTENSIONS array, and JsAdapter::can_handle() for 'cjs' support."
    );
}

#[test]
fn test_cjs_file_detected_as_javascript_language() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("app.cjs"), "// CommonJS module\n").unwrap();

    // We test through analyze() since discover_source_files is private.
    // analyze() with no language filter should discover the .cjs file.
    let config = Config::load(tmp.path(), None).unwrap();
    let result = crate::analyze(tmp.path(), &config, &[]).unwrap();

    // The fact that analyze succeeds on a directory with only .cjs files
    // proves the file was discovered and routed to the JS adapter.
    assert!(
        result
            .manifest
            .metadata
            .languages
            .contains(&"javascript".to_string()),
        "Language for .cjs must be 'javascript'. Detected: {:?}",
        result.manifest.metadata.languages
    );
}
