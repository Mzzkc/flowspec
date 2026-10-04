//! Type annotation origin is ordinary IR evidence, independent of probe compilation.
use flowspec::parser::{python::PythonAdapter, LanguageAdapter};
#[test]
fn ordinary_adapter_preserves_annotation_and_value_origins() {
    let source = "from types_module import Widget\ndef api(value: Widget):\n    return Widget\n";
    let parsed = PythonAdapter::new()
        .parse_file(std::path::Path::new("module.py"), source)
        .unwrap();
    let refs:Vec<_>=parsed.references.iter().filter(|r|matches!(&r.resolution,flowspec::parser::ir::ResolutionStatus::Partial(s) if s=="attribute_access:Widget")).map(|r|serde_json::to_value(r.kind).unwrap()).collect();
    assert!(refs.iter().any(|r| r == "Annotation"), "{refs:?}");
    assert!(refs.iter().any(|r| r == "Read"), "{refs:?}");
}
