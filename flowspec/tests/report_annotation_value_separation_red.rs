//! An annotation must not fabricate a runtime read at the same syntax node.
use flowspec::parser::{ir::ReferenceKind, python::PythonAdapter, LanguageAdapter};

#[test]
fn annotation_node_has_no_fictitious_value_read_and_runtime_read_survives() {
    let parsed = PythonAdapter::new()
        .parse_file(
            std::path::Path::new("module.py"),
            "from peer import Widget\ndef api(value: Widget):\n    return Widget\n",
        )
        .unwrap();
    let annotation = parsed
        .references
        .iter()
        .find(|reference| {
            reference.kind == ReferenceKind::Annotation
                && matches!(&reference.resolution, flowspec::parser::ir::ResolutionStatus::Partial(s) if s == "attribute_access:Widget")
        })
        .expect("genuine annotation reference");
    assert!(
        parsed.references.iter().all(|reference| {
            reference.kind != ReferenceKind::Read || reference.location != annotation.location
        }),
        "annotation syntax is not a runtime value read: {:?}",
        parsed.references
    );
    assert!(
        parsed.references.iter().any(|reference| {
            reference.kind == ReferenceKind::Read && reference.location.line == 3
        }),
        "actual returned value must remain a Read"
    );
}
