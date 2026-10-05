use flowspec::{analyze, Config, Graph, trace_flows_from};
use flowspec::parser::ir::*;
use std::path::PathBuf;

fn symbol(name: &str, import: bool) -> Symbol {
    Symbol { id: SymbolId::default(), kind: SymbolKind::Function,
        name:name.into(), qualified_name:format!("{name}.py::{name}"), visibility:Visibility::Private,
        signature:None,location:Location { file:PathBuf::from(format!("{name}.py")),line:1,column:1,end_line:1,end_column:2 },
        resolution:ResolutionStatus::Resolved,scope:ScopeId::default(),annotations:if import {vec!["import".into()]} else {vec![]} }
}
fn edge(g:&mut Graph,from:SymbolId,to:SymbolId,kind:ReferenceKind) {
    g.add_reference(Reference {id:ReferenceId::default(),from,to,kind,location:Location {file:PathBuf::from("main.py"),line:2,column:1,end_line:2,end_column:4},resolution:ResolutionStatus::Resolved});
}
#[test]
fn cross_file_trace_retains_the_actual_import_reference_hop() {
    let mut g=Graph::new();let main=g.add_symbol(symbol("main",false));
    let proxy=g.add_symbol(symbol("alias",true));let target=g.add_symbol(symbol("real",false));
    edge(&mut g,main,proxy,ReferenceKind::Call);edge(&mut g,proxy,target,ReferenceKind::Read);
    let paths=trace_flows_from(&g,main,20);
    assert_eq!(paths.len(),1);
    assert_eq!(paths[0].steps.iter().map(|s|(s.symbol,s.edge_kind)).collect::<Vec<_>>(),vec![(proxy,EdgeKind::Calls),(target,EdgeKind::References)],"An import proxy cannot be collapsed into a call edge that does not exist");
}
#[test]
fn actual_typed_producer_transformer_and_consumer_are_not_empty() {
    let p=tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("model.py"),"class Raw: pass\nclass Cooked: pass\ndef make() -> Raw:\n    return Raw()\ndef transform(value: Raw) -> Cooked:\n    return Cooked()\ndef consume(value: Cooked):\n    return value\ndef main():\n    consume(transform(make()))\n").unwrap();
    let result=analyze(p.path(),&Config::default(),&[]).unwrap();
    let raw=result.manifest.type_flows.iter().find(|f|f.type_name.ends_with("::Raw")).expect("resolved runtime construction and annotation references must produce a Raw type flow");
    assert!(raw.created_at.iter().any(|s|s.ends_with("::make")));
    assert!(raw.transformed_to.iter().any(|t|t.target_type.ends_with("::Cooked")&&t.via.ends_with("::transform")));
    let cooked=result.manifest.type_flows.iter().find(|f|f.type_name.ends_with("::Cooked")).unwrap();
    assert!(cooked.consumed_by.iter().any(|s|s.ends_with("::consume")));
}
