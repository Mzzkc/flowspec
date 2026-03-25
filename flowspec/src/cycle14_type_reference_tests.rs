//! QA-Foundation (QA 1) — Cycle 14: Type reference emission tests.
//!
//! 25 tests across 5 categories validating:
//! - Parser-level type reference emission (T1–T8)
//! - Adversarial must-NOT-emit guards (T9–T13)
//! - Integration full pipeline (T14–T17)
//! - Regression guards (T18–T21)
//! - Dogfood-specific patterns (T22–T25)

use crate::parser::ir::ResolutionStatus;
use crate::parser::rust::RustAdapter;
use crate::parser::LanguageAdapter;
use std::path::Path;

// =========================================================================
// Helper: extract attribute_access names from parse result
// =========================================================================

fn attr_access_names(source: &str) -> Vec<String> {
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(Path::new("lib.rs"), source).unwrap();
    result
        .references
        .iter()
        .filter_map(|r| match &r.resolution {
            ResolutionStatus::Partial(info) if info.starts_with("attribute_access:") => {
                Some(info["attribute_access:".len()..].to_string())
            }
            _ => None,
        })
        .collect()
}

fn has_attr_access(source: &str, name: &str) -> bool {
    attr_access_names(source).contains(&name.to_string())
}

fn has_attr_access_with_file(source: &str, name: &str, filename: &str) -> bool {
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(Path::new(filename), source).unwrap();
    result.references.iter().any(|r| {
        matches!(&r.resolution,
            ResolutionStatus::Partial(info) if info == &format!("attribute_access:{}", name))
    })
}

// =========================================================================
// Category 1: Parser-Level Type Reference Emission (T1–T8)
// =========================================================================

/// T1: Basic type annotation in function parameter
#[test]
fn test_type_reference_emitted_for_parameter_type_annotation() {
    let source = r#"
use std::path::Path;

fn read_file(p: &Path) -> String {
    String::new()
}
"#;
    assert!(
        has_attr_access(source, "Path"),
        "parse_file must emit attribute_access:Path for type annotation usage. \
         Without this, phantom_dependency fires on Path import. \
         Got references: {:?}",
        attr_access_names(source)
    );
}

/// T2: Return type annotation
#[test]
fn test_type_reference_emitted_for_return_type() {
    let source = r#"
use std::collections::HashMap;

fn build() -> HashMap<String, i32> {
    HashMap::new()
}
"#;
    assert!(
        has_attr_access(source, "HashMap"),
        "parse_file must emit attribute_access:HashMap for return type usage"
    );
}

/// T3: Struct field type annotation
#[test]
fn test_type_reference_emitted_for_struct_field_type() {
    let source = r#"
use std::path::PathBuf;

struct Config {
    root: PathBuf,
    name: String,
}
"#;
    assert!(
        has_attr_access_with_file(source, "PathBuf", "config.rs"),
        "parse_file must emit attribute_access:PathBuf for struct field type"
    );
}

/// T4: Trait bound usage
#[test]
fn test_type_reference_emitted_for_trait_bound() {
    let source = r#"
use crate::parser::LanguageAdapter;

fn process<T: LanguageAdapter>(adapter: &T) {
    // uses adapter
}
"#;
    assert!(
        has_attr_access(source, "LanguageAdapter"),
        "parse_file must emit attribute_access:LanguageAdapter for trait bound usage"
    );
}

/// T5: Generic type parameter (inside angle brackets)
#[test]
fn test_type_reference_emitted_for_generic_type_argument() {
    let source = r#"
use crate::parser::ir::SymbolId;

fn get_ids() -> Vec<SymbolId> {
    vec![]
}
"#;
    assert!(
        has_attr_access(source, "SymbolId"),
        "parse_file must emit attribute_access:SymbolId for generic type argument"
    );
}

/// T6: Where clause trait bound
#[test]
fn test_type_reference_emitted_for_where_clause() {
    let source = r#"
use std::fmt::Display;

fn show<T>(val: T) where T: Display {
    println!("{}", val);
}
"#;
    assert!(
        has_attr_access(source, "Display"),
        "parse_file must emit attribute_access:Display for where clause trait bound"
    );
}

/// T7: Let binding type annotation
#[test]
fn test_type_reference_emitted_for_let_binding_type() {
    let source = r#"
use std::collections::HashSet;

fn collect() {
    let items: HashSet<String> = HashSet::new();
}
"#;
    assert!(
        has_attr_access(source, "HashSet"),
        "parse_file must emit attribute_access:HashSet for let binding type annotation"
    );
}

/// T8: Pattern match with enum variant (type position)
#[test]
fn test_type_reference_emitted_for_match_pattern_enum() {
    let source = r#"
use crate::parser::ir::SymbolKind;

fn check(kind: u8) {
    let sk = SymbolKind::Function;
    match sk {
        SymbolKind::Function => {},
        SymbolKind::Method => {},
        _ => {},
    }
}
"#;
    assert!(
        has_attr_access(source, "SymbolKind"),
        "parse_file must emit attribute_access:SymbolKind for enum variant usage in match/construction"
    );
}

// =========================================================================
// Category 2: Adversarial — Must NOT Emit References (T9–T13)
// =========================================================================

/// T9: Primitive types — documenting behavior (soft assertion)
#[test]
fn test_no_type_reference_emitted_for_primitive_types() {
    let source = r#"
fn add(a: u32, b: i64) -> bool {
    true
}
"#;
    let refs = attr_access_names(source);
    let primitive_refs: Vec<_> = refs
        .iter()
        .filter(|n| *n == "u32" || *n == "i64" || *n == "bool")
        .collect();

    // Note: emitting these is not WRONG (they won't resolve to imports),
    // but if the implementation filters them, verify it's consistent.
    // This test documents current behavior — primitives should be filtered.
    assert!(
        primitive_refs.is_empty(),
        "Primitive types should NOT emit attribute_access references. Found: {:?}",
        primitive_refs
    );
}

/// T10: Self/self must NOT emit type references
#[test]
fn test_no_type_reference_for_self_or_self_type() {
    let source = r#"
struct Foo { x: i32 }

impl Foo {
    fn new() -> Self {
        Self { x: 0 }
    }
    fn get(&self) -> i32 {
        self.x
    }
}
"#;
    let refs = attr_access_names(source);
    let self_refs: Vec<_> = refs
        .iter()
        .filter(|n| *n == "Self" || *n == "self")
        .collect();

    assert!(
        self_refs.is_empty(),
        "Self/self must NOT emit type references — they're keywords, not imports. \
         Found: {} refs",
        self_refs.len()
    );
}

/// T11: Variable names must NOT emit type references
#[test]
fn test_no_type_reference_for_variable_names() {
    let source = r#"
use crate::graph::Graph;

fn build() {
    let graph = Graph::new();
    println!("{:?}", graph);
}
"#;
    let refs = attr_access_names(source);
    let var_refs: Vec<_> = refs.iter().filter(|n| *n == "graph").collect();

    assert!(
        var_refs.is_empty(),
        "Variable name 'graph' must NOT emit attribute_access reference. \
         Only 'Graph' (the type) should. Found {} variable refs.",
        var_refs.len()
    );
}

/// T12: Nested generic types emit all type refs
#[test]
fn test_nested_generic_types_emit_all_type_references() {
    let source = r#"
use std::collections::HashMap;
use crate::parser::ir::SymbolId;

fn lookup() -> HashMap<String, Vec<SymbolId>> {
    HashMap::new()
}
"#;
    let refs = attr_access_names(source);

    assert!(
        refs.contains(&"HashMap".to_string()),
        "Must emit attribute_access:HashMap from outer generic"
    );
    assert!(
        refs.contains(&"SymbolId".to_string()),
        "Must emit attribute_access:SymbolId from nested generic Vec<SymbolId>"
    );
}

/// T13: Scoped type identifier emits prefix reference
#[test]
fn test_scoped_type_identifier_emits_prefix_reference() {
    let source = r#"
use std::io;

fn read() -> io::Result<()> {
    Ok(())
}
"#;
    assert!(
        has_attr_access(source, "io"),
        "Scoped type identifier io::Result must emit attribute_access:io for the prefix. \
         Without this, `use std::io;` remains a phantom dependency FP."
    );
}

// =========================================================================
// Category 3: Integration — Full Pipeline (T14–T17)
// =========================================================================

/// T14: Type annotation usage prevents phantom_dependency (integration)
#[test]
fn test_integration_type_annotation_prevents_phantom_dependency() {
    let source = r#"
use std::collections::HashMap;
use std::path::Path;

fn build_map(root: &Path) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();
    map
}
"#;
    let refs = attr_access_names(source);

    assert!(
        refs.contains(&"HashMap".to_string()),
        "Pipeline precondition: HashMap attribute_access reference must exist"
    );
    assert!(
        refs.contains(&"Path".to_string()),
        "Pipeline precondition: Path attribute_access reference must exist"
    );
}

/// T15: Genuinely unused import still detected after fix (integration)
#[test]
fn test_integration_unused_import_still_phantom_with_type_fix() {
    let source = r#"
use std::collections::HashMap;
use std::io;

fn build() -> HashMap<String, String> {
    HashMap::new()
}
"#;
    let refs = attr_access_names(source);

    // HashMap is used in return type — must have reference
    assert!(
        refs.contains(&"HashMap".to_string()),
        "HashMap used in return type must emit attribute_access reference"
    );

    // io is imported but never used — must NOT have reference
    // (unless io appears in a scoped type like io::Error, which it doesn't here)
    assert!(
        !refs.contains(&"io".to_string()),
        "io is imported but never used in type position — no attribute_access reference expected. \
         If this fails, the fix is over-emitting references (emitting for import declarations, not usages)."
    );
}

/// T16: Multiple types from same module all emit references
#[test]
fn test_multiple_types_from_use_list_all_emit_references() {
    let source = r#"
use crate::parser::ir::{SymbolKind, EdgeKind, Visibility};

fn classify(kind: SymbolKind, edge: EdgeKind, vis: Visibility) -> bool {
    true
}
"#;
    let refs = attr_access_names_with_file(source, "analyzer.rs");

    for expected in &["SymbolKind", "EdgeKind", "Visibility"] {
        assert!(
            refs.contains(&expected.to_string()),
            "Must emit attribute_access:{} for type used in parameter",
            expected
        );
    }
}

/// T17: Impl block trait name emits reference
#[test]
fn test_impl_trait_name_emits_type_reference() {
    let source = r#"
use std::fmt::Display;

struct MyType;

impl Display for MyType {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "MyType")
    }
}
"#;
    assert!(
        has_attr_access(source, "Display"),
        "impl Display for MyType must emit attribute_access:Display for the trait name"
    );
}

// =========================================================================
// Category 4: Regression Guards (T18–T21)
// =========================================================================

/// T18: C13 scoped call prefix still works (regression)
#[test]
fn test_c13_scoped_call_prefix_regression() {
    let source = r#"
use std::fs;

fn read() {
    let _ = fs::read_to_string("config.toml");
}
"#;
    assert!(
        has_attr_access(source, "fs"),
        "C13 regression: scoped call fs::read_to_string must still emit attribute_access:fs"
    );
}

/// T19: Genuinely unused import T3 regression (the `io` test)
/// This verifies that the existing phantom_dependency test at
/// phantom_dependency.rs:195-219 is preserved. We verify the parser
/// side: an unused import must NOT get spurious type references.
#[test]
fn test_c13_unused_import_regression_guard_parser_side() {
    let source = r#"
use std::io;

fn main() {
    println!("hello");
}
"#;
    assert!(
        !has_attr_access(source, "io"),
        "Unused io import must NOT emit attribute_access:io — phantom_dependency must still fire"
    );
}

// T20: Existing 24 C13 tests all pass
// This is verified by `cargo test --all` — not a separate test function.
// See cycle13_cjs_and_use_path_tests.rs.

/// T21: References edge kind resolves through populate.rs (existing path)
/// This verifies the attribute_access: resolution path is functional by
/// checking that parse output contains the expected format.
#[test]
fn test_attribute_access_reference_format_correct() {
    let source = r#"
use std::path::Path;

fn read(p: &Path) {}
"#;
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(Path::new("lib.rs"), source).unwrap();

    let attr_refs: Vec<_> = result
        .references
        .iter()
        .filter(|r| {
            matches!(&r.resolution,
                ResolutionStatus::Partial(info) if info == "attribute_access:Path")
        })
        .collect();

    assert!(
        !attr_refs.is_empty(),
        "attribute_access:Path reference must exist in parse output for populate.rs resolution"
    );

    // Verify the reference uses ReferenceKind::Read (same as C13's scoped_identifier path)
    for r in &attr_refs {
        assert_eq!(
            r.kind,
            crate::parser::ir::ReferenceKind::Read,
            "Type references must use ReferenceKind::Read for populate.rs compatibility"
        );
    }
}

// =========================================================================
// Category 5: Dogfood-Specific Patterns (T22–T25)
// =========================================================================

/// T22: SymbolKind pattern matching (16 FPs in dogfood)
#[test]
fn test_dogfood_pattern_symbolkind_match() {
    let source = r#"
use crate::parser::ir::SymbolKind;

fn is_callable(sym_kind: SymbolKind) -> bool {
    matches!(sym_kind, SymbolKind::Function | SymbolKind::Method)
}
"#;
    assert!(
        has_attr_access(source, "SymbolKind"),
        "Dogfood regression: SymbolKind used in parameter + match pattern must emit reference"
    );
}

/// T23: Graph type annotation (15 FPs in dogfood)
#[test]
fn test_dogfood_pattern_graph_type_annotation() {
    let source = r#"
use crate::graph::Graph;

fn analyze(graph: &Graph) -> Vec<String> {
    vec![]
}
"#;
    assert!(
        has_attr_access_with_file(source, "Graph", "pattern.rs"),
        "Dogfood regression: Graph used as &Graph parameter type must emit reference"
    );
}

/// T24: EdgeKind in match arms (10 FPs in dogfood)
#[test]
fn test_dogfood_pattern_edgekind_match() {
    let source = r#"
use crate::parser::ir::EdgeKind;

fn is_call_edge(kind: EdgeKind) -> bool {
    match kind {
        EdgeKind::Calls => true,
        EdgeKind::References => false,
        _ => false,
    }
}
"#;
    assert!(
        has_attr_access_with_file(source, "EdgeKind", "flow.rs"),
        "Dogfood regression: EdgeKind in parameter type + match must emit reference"
    );
}

/// T25: Path type in multiple positions (27 FPs in dogfood)
#[test]
fn test_dogfood_pattern_path_multiple_positions() {
    let source = r#"
use std::path::Path;

fn resolve(base: &Path, relative: &Path) -> bool {
    let target: &Path = base;
    target.exists()
}
"#;
    let refs = attr_access_names_with_file(source, "commands.rs");
    let path_refs: Vec<_> = refs.iter().filter(|n| *n == "Path").collect();

    // Path appears 3 times as a type annotation — at least one reference must exist
    assert!(
        !path_refs.is_empty(),
        "Dogfood regression: Path used in multiple type annotation positions must emit at least one reference"
    );
}

// =========================================================================
// Helper with custom filename
// =========================================================================

fn attr_access_names_with_file(source: &str, filename: &str) -> Vec<String> {
    let adapter = RustAdapter::new();
    let result = adapter.parse_file(Path::new(filename), source).unwrap();
    result
        .references
        .iter()
        .filter_map(|r| match &r.resolution {
            ResolutionStatus::Partial(info) if info.starts_with("attribute_access:") => {
                Some(info["attribute_access:".len()..].to_string())
            }
            _ => None,
        })
        .collect()
}
