//! Visiting an annotation through two AST passes must not create two resolver obligations.
use flowspec::parser::{python::PythonAdapter, LanguageAdapter};
#[test]
fn one_actual_annotation_node_has_one_resolver_origin() {
    let result = PythonAdapter::new()
        .parse_file(
            std::path::Path::new("module.py"),
            "class Widget:\n    pass\ndef api(value: Widget):\n    pass\n",
        )
        .unwrap();
    let origins:Vec<_>=result.references.iter().filter(|r|serde_json::to_value(r.kind).unwrap()=="Annotation" && matches!(&r.resolution,flowspec::parser::ir::ResolutionStatus::Partial(s) if s=="attribute_access:Widget")).collect();
    assert_eq!(origins.len(), 1, "{origins:?}");
}
