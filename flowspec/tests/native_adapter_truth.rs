use flowspec::parser::{
    ir::{SymbolKind, Visibility},
    javascript::JsAdapter,
    LanguageAdapter,
};
use flowspec::registry::Registry;
use std::path::Path;

#[test]
fn native_generic_signature_and_original_end_column() {
    let source = "export function identity<T>(value: T): T { return value; }";
    let ir = JsAdapter::new()
        .parse_file(Path::new("typed.ts"), source)
        .unwrap();
    let symbol = ir.symbols.iter().find(|s| s.name == "identity").unwrap();
    assert_eq!(symbol.visibility, Visibility::Public);
    assert_eq!(symbol.signature.as_deref(), Some("(value: T)"));
    assert_eq!(symbol.location.end_column as usize, source.len() + 1);
}
#[test]
fn native_ts_registry_factory_owns_language_identity() {
    let registry = Registry::default();
    let descriptor = registry.language("typescript").unwrap();
    let adapter = descriptor.factory.unwrap()();
    assert_eq!(adapter.language_name(), "typescript");
    assert!(descriptor
        .grammar_version
        .starts_with("tree-sitter-typescript/"));
}
#[test]
fn native_implements_and_ambient_class_each_have_one_identity() {
    for source in [
        "class App implements Config { start(): void {} }",
        "declare class App { start(): void; }",
    ] {
        let ir = JsAdapter::new()
            .parse_file(Path::new("app.ts"), source)
            .unwrap();
        let apps: Vec<_> = ir
            .symbols
            .iter()
            .filter(|s| s.name == "App" && s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].location.end_column as usize, source.len() + 1);
    }
}
#[test]
fn native_tsx_preserves_full_function_original_range() {
    let source =
        "export function View(props: { title: string }) { return <h1>{props.title}</h1>; }";
    let ir = JsAdapter::new()
        .parse_file(Path::new("view.tsx"), source)
        .unwrap();
    let view = ir.symbols.iter().find(|s| s.name == "View").unwrap();
    assert_eq!(view.location.end_column as usize, source.len() + 1);
    assert_eq!(
        view.signature.as_deref(),
        Some("(props: { title: string })")
    );
}
