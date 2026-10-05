use flowspec::parser::ir::{EdgeKind, SymbolKind};
use flowspec::{analyze, Config};
use serde_json::Value;
fn run(source: &str, ext: &str, policy: &str) -> flowspec::AnalysisResult {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join(format!("source.{ext}")), source).unwrap();
    let path = root.path().join("explicit.yaml");
    std::fs::write(&path, policy).unwrap();
    let c = Config::load(root.path(), Some(&path)).unwrap();
    analyze(root.path(), &c, &[]).unwrap()
}
#[test]
fn rust_receiver_binding_requires_the_actual_parameter_owner() {
    for ty in ["Graph", "Other"] {
        let r = run(&format!("pub struct Graph; pub struct Other; impl Graph {{ pub fn add_symbol(&self) -> bool {{ true }} }} pub fn populate(graph: &{ty}) -> bool {{ graph.add_symbol() }}"), "rs", "");
        let (method, _) = r
            .graph
            .all_symbols()
            .find(|(_, s)| s.kind == SymbolKind::Method && s.name == "add_symbol")
            .unwrap();
        let (caller, _) = r
            .graph
            .all_symbols()
            .find(|(_, s)| s.name == "populate")
            .unwrap();
        let bound = r
            .graph
            .edges_from(caller)
            .iter()
            .any(|e| e.kind == EdgeKind::Calls && e.target == method);
        assert_eq!(
            bound,
            ty == "Graph",
            "receiver owner must determine exact edge, never unique method spelling"
        );
        if ty == "Other" {
            assert!(!r.graph.unresolved_calls().is_empty());
        }
    }
}
#[test]
fn rust_shadowed_receiver_retains_unknown_instead_of_consuming_outer_owner() {
    let r = run("pub struct Graph; impl Graph { pub fn add_symbol(&self) {} } pub fn populate(graph: &Graph) { let graph = unknown(); graph.add_symbol(); }", "rs", "");
    let (method, _) = r
        .graph
        .all_symbols()
        .find(|(_, s)| s.name == "add_symbol")
        .unwrap();
    assert!(r
        .graph
        .edges_to(method)
        .iter()
        .all(|e| e.kind != EdgeKind::Calls));
}
fn findings<'a>(r: &'a Value, pattern: &str) -> Vec<&'a Value> {
    r["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["pattern_id"] == pattern)
        .collect()
}
#[test]
fn four_language_declared_normal_exit_peer_paths_have_positive_and_satisfied_rivals() {
    for (ext, source) in [
        ("py", "def cleanup():\n    return True\ndef left(x):\n    if x:\n        cleanup()\n    return x\ndef right(x):\n    cleanup()\n    return x\n"),
        ("js", "function cleanup() { return true; } function left(x) { if (x) { cleanup(); } return x; } function right(x) { cleanup(); return x; }"),
        ("ts", "function cleanup(): boolean { return true; } function left(x: boolean): boolean { if (x) { cleanup(); } return x; } function right(x: boolean): boolean { cleanup(); return x; }"),
        ("rs", "fn cleanup() -> bool { true } fn left(x: bool) -> bool { if x { cleanup(); } return x; } fn right(x: bool) -> bool { cleanup(); return x; }")
    ] {
        let policy = format!("semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.{ext}::left, source.{ext}::right]\n      required_operations: [source.{ext}::cleanup]\n      exit_scope: normal\n");
        let r=run(source,ext,&policy).manifest.authority.unwrap();
        let fs=findings(&r,"partial_wiring");
        assert_eq!(fs.len(),1,"missing operation on one normal-return branch: {r}");
        assert_eq!(fs[0]["subject"]["name"],"left");
        assert_eq!(fs[0]["evidence"][0]["cfg_exit_scope"],"normal");
        let fixed=if ext == "py" {source.replace("    if x:\n        cleanup()", "    cleanup()")} else {source.replace("if (x) { cleanup(); }", "cleanup();").replace("if x { cleanup(); }", "cleanup();")};
        let good=run(&fixed,ext,&policy).manifest.authority.unwrap();
        assert!(findings(&good,"partial_wiring").is_empty(),"satisfied rival: {good}");
        assert!(good["losses"].as_array().unwrap().iter().all(|l|l["reason"] != "policy_operation_unavailable" || !l["affected_patterns"].as_array().unwrap().iter().any(|p|p == "partial_wiring")));
    }
}
#[test]
fn peer_policy_missing_binding_and_unsupported_exit_scope_refuse_clean_claims() {
    let policy="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::missing]\n      exit_scope: normal\n";
    let r = run("function left() {} function right() {}", "js", policy)
        .manifest
        .authority
        .unwrap();
    assert!(findings(&r, "partial_wiring").is_empty());
    assert!(r["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "policy_binding_unresolved"));
    let root = tempfile::tempdir().unwrap();
    let p = root.path().join("policy.yaml");
    std::fs::write(&p, policy.replace("normal", "invented")).unwrap();
    assert!(Config::load(root.path(), Some(&p)).is_err());
}
#[test]
fn normal_exit_peer_cfg_follows_early_returns_and_transitive_calls() {
    let d=tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("source.js"),"import { cleanup } from './peer.js'; function relay() { cleanup(); } function left(x) { if (x) { return x; } relay(); return x; } function right(x) { relay(); return x; }").unwrap();
    std::fs::write(d.path().join("peer.js"),"export function cleanup() { return true; }").unwrap();
    let p=d.path().join("policy.yaml");std::fs::write(&p,"semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [peer.js::cleanup]\n      exit_scope: normal\n").unwrap();
    let c=Config::load(d.path(),Some(&p)).unwrap();
    let debug = analyze(d.path(),&c,&[]).unwrap();
    for f in debug.graph.semantic_files() { eprintln!("CONTROL {} {:?}",f.path.display(),f.normal_control); }
    for (id,s) in debug.graph.all_symbols() { for e in debug.graph.edges_from(id) { if e.kind == EdgeKind::Calls {eprintln!("CALL {} -> {:?} {:?}",s.qualified_name,debug.graph.get_symbol(e.target).map(|s| &s.qualified_name),e.reference_id.and_then(|i|debug.graph.get_reference(i)));} } }
    for a in [analyze(d.path(),&c,&[]).unwrap().manifest.authority.unwrap(),flowspec::diagnose_report(d.path(),&c,&[],&flowspec::ReportOptions::default()).unwrap().authority.unwrap()] {
        let f=findings(&a,"partial_wiring");assert_eq!(f.len(),1,"normal path and exact cross-file/transitive target: {a}");assert_eq!(f[0]["subject"]["name"],"left");
        assert_eq!(f[0]["evidence"][0]["binding_targets"][0]["path_key"],"706565722e6a73");
    }
}
#[test]
fn loops_and_unresolved_dispatch_keep_peer_closure_located_incomplete() {
    for body in ["while (x) { cleanup(); }", "unknown();", "x.cleanup();"] {
        let source=format!("function cleanup() {{ return true; }} function left(x) {{ {body} return x; }} function right(x) {{ cleanup(); return x; }}");
        let p="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::cleanup]\n      exit_scope: normal\n";
        let a=run(&source,"js",p).manifest.authority.unwrap();assert!(findings(&a,"partial_wiring").is_empty());
        assert!(a["losses"].as_array().unwrap().iter().any(|l|l["reason"]=="policy_control_unresolved" && !l["ranges"].as_array().unwrap().is_empty()));
    }
}
#[test]
fn literal_true_branch_does_not_invent_an_impossible_missing_operation_path() {
    let p="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::cleanup]\n      exit_scope: normal\n";
    let a=run("function cleanup() { return true; } function left(x) { if (true) { cleanup(); } return x; } function right(x) { cleanup(); return x; }","js",p).manifest.authority.unwrap();assert!(findings(&a,"partial_wiring").is_empty(),"literal-known CFG branch is not a feasible rival: {a}");
}
