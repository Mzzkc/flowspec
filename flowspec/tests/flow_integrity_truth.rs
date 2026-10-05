use flowspec::parser::ir::*;
use flowspec::{analyze, trace_flows_from, Config, Graph};
use std::path::PathBuf;

fn symbol(name: &str, import: bool) -> Symbol {
    Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Function,
        name: name.into(),
        qualified_name: format!("{name}.py::{name}"),
        visibility: Visibility::Private,
        signature: None,
        location: Location {
            file: PathBuf::from(format!("{name}.py")),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 2,
        },
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: if import {
            vec!["import".into()]
        } else {
            vec![]
        },
    }
}
fn edge(g: &mut Graph, from: SymbolId, to: SymbolId, kind: ReferenceKind) {
    g.add_reference(Reference {
        id: ReferenceId::default(),
        from,
        to,
        kind,
        location: Location {
            file: PathBuf::from("main.py"),
            line: 2,
            column: 1,
            end_line: 2,
            end_column: 4,
        },
        resolution: ResolutionStatus::Resolved,
    });
}
#[test]
fn cross_file_trace_retains_the_actual_import_reference_hop() {
    let mut g = Graph::new();
    let main = g.add_symbol(symbol("main", false));
    let proxy = g.add_symbol(symbol("alias", true));
    let target = g.add_symbol(symbol("real", false));
    edge(&mut g, main, proxy, ReferenceKind::Call);
    edge(&mut g, proxy, target, ReferenceKind::Read);
    let paths = trace_flows_from(&g, main, 20);
    assert_eq!(paths.len(), 1);
    assert_eq!(
        paths[0]
            .steps
            .iter()
            .map(|s| (s.symbol, s.edge_kind))
            .collect::<Vec<_>>(),
        vec![(proxy, EdgeKind::Calls), (target, EdgeKind::References)],
        "An import proxy cannot be collapsed into a call edge that does not exist"
    );
}
#[test]
fn actual_typed_producer_transformer_and_consumer_are_not_empty() {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("model.py"),"class Raw: pass\nclass Cooked: pass\ndef make() -> Raw:\n    return Raw()\ndef transform(value: Raw) -> Cooked:\n    return Cooked(value)\ndef consume(value: Cooked):\n    return value\ndef main():\n    consume(transform(make()))\n").unwrap();
    let result = analyze(p.path(), &Config::default(), &[]).unwrap();
    let raw = result
        .manifest
        .type_flows
        .iter()
        .find(|f| f.type_name.ends_with("::Raw"))
        .expect(
            "resolved runtime construction and annotation references must produce a Raw type flow",
        );
    assert!(raw.created_at.iter().any(|s| s.ends_with("::make")));
    assert!(raw
        .transformed_to
        .iter()
        .any(|t| t.target_type.ends_with("::Cooked") && t.via.ends_with("::transform")));
    let cooked = result
        .manifest
        .type_flows
        .iter()
        .find(|f| f.type_name.ends_with("::Cooked"))
        .unwrap();
    assert!(cooked.consumed_by.iter().any(|s| s.ends_with("::consume")));
}

#[test]
fn annotation_only_and_shadowed_constructor_are_not_runtime_type_flows() {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("model.py"),"class Raw: pass\ndef annotation_only(value: Raw):\n    pass\ndef shadowed(Raw):\n    return Raw()\n").unwrap();
    let result = analyze(p.path(), &Config::default(), &[]).unwrap();
    assert!(
        result
            .manifest
            .type_flows
            .iter()
            .all(|f| f.created_at.is_empty() && f.consumed_by.is_empty()),
        "Annotation reads and parameter-shadowed calls are not runtime type operations"
    );
}
#[test]
fn discarded_conversion_is_not_a_returned_type_transformation() {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("model.py"),"class Raw: pass\nclass Cooked: pass\ndef transform(value: Raw) -> Cooked:\n    Cooked(value)\n    return Cooked()\n").unwrap();
    let result = analyze(p.path(), &Config::default(), &[]).unwrap();
    assert!(result
        .manifest
        .type_flows
        .iter()
        .all(|f| f.transformed_to.is_empty()));
}
#[test]
fn trace_depth_is_explicit_and_terminal_at_limit_is_complete() {
    let mut g = Graph::new();
    let a = g.add_symbol(symbol("a", false));
    let b = g.add_symbol(symbol("b", false));
    let c = g.add_symbol(symbol("c", false));
    edge(&mut g, a, b, ReferenceKind::Call);
    edge(&mut g, b, c, ReferenceKind::Call);
    let partial = flowspec::analyzer::flow::trace_report(&g, a, 1, false);
    assert!(partial.incomplete);
    assert_eq!(partial.continuation.len(), 1);
    assert_eq!(partial.paths[0].steps.len(), 1);
    let full = flowspec::analyzer::flow::trace_report(&g, a, 2, false);
    assert!(!full.incomplete);
    assert!(full.continuation.is_empty());
    assert_eq!(full.paths[0].steps.len(), 2);
}

#[test]
fn property_names_and_keyword_labels_do_not_consume_a_same_named_parameter() {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("model.py"), "class Raw: pass\ndef helper(value):\n    return value\ndef property_only(value: Raw):\n    return Raw.value\ndef keyword_only(value: Raw):\n    return helper(value=Raw())\n").unwrap();
    let result = analyze(p.path(), &Config::default(), &[]).unwrap();
    assert!(
        result
            .manifest
            .type_flows
            .iter()
            .all(|flow| flow.consumed_by.iter().all(
                |site| !site.ends_with("::property_only") && !site.ends_with("::keyword_only")
            )),
        "member names and keyword labels are not reads of the parameter binding"
    );
}
