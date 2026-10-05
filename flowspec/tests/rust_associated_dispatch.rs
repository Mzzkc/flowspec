use flowspec::parser::ir::{EdgeKind, SymbolKind};
use flowspec::{analyze, Config};
use std::fs;

#[test]
fn rust_associated_calls_and_callback_values_keep_the_declared_type_owner() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("entry.rs"),
        r#"
struct Alpha;
struct Beta;
impl Alpha { fn build() -> Self { Alpha } fn score(&self) -> i32 { 1 } }
impl Beta { fn build() -> Self { Beta } fn score(&self) -> i32 { 2 } }
fn main() {
    let _a = Alpha::build();
    let _b = Beta::build();
    let _scores: Vec<_> = [Alpha].iter().map(Alpha::score).collect();
    let _rival: Vec<_> = [Beta].iter().map(Beta::score).collect();
}
"#,
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    for (id, symbol) in result
        .graph
        .all_symbols()
        .filter(|(_, s)| s.kind == SymbolKind::Method)
    {
        assert!(
            result
                .graph
                .edges_to(id)
                .iter()
                .any(|edge| matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)),
            "explicit associated consumer missing for {}",
            symbol.qualified_name
        );
    }
}

#[test]
fn rust_unknown_associated_owner_cannot_bind_to_an_unrelated_method() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("entry.rs"),
        "struct Local; impl Local { fn build() -> Self { Local } } fn main() { Foreign::build(); }",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    let (id, _) = result
        .graph
        .all_symbols()
        .find(|(_, s)| s.kind == SymbolKind::Method && s.name == "build")
        .unwrap();
    assert!(
        !result
            .graph
            .edges_to(id)
            .iter()
            .any(|edge| matches!(edge.kind, EdgeKind::Calls | EdgeKind::References)),
        "foreign associated owner must remain unresolved"
    );
}
