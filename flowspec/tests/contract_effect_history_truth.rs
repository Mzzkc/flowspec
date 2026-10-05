use flowspec::{analyze, Config};
use serde_json::Value;
fn run(source: &str, ext: &str, policy: &str) -> Value {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join(format!("source.{ext}")), source).unwrap();
    let p=d.path().join("policy.yaml"); std::fs::write(&p,policy).unwrap();
    let c=Config::load(d.path(),Some(&p)).unwrap();
    let a=analyze(d.path(),&c,&[]).unwrap().manifest.authority.unwrap();
    let b=flowspec::diagnose_report(d.path(),&c,&[],&flowspec::ReportOptions::default()).unwrap().authority.unwrap();
    assert_eq!(a["findings"], b["findings"]);
    a
}
fn count(a:&Value,p:&str)->usize { a["findings"]["all"].as_array().unwrap().iter().filter(|f|f["pattern_id"]==p).count() }
#[test]
fn declared_all_exit_peer_exception_requires_operation_before_throw() {
    let p="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::cleanup]\n";
    let a=run("function cleanup() { return true; } function left() { throw true; } function right() { cleanup(); return true; }","js",p);
    assert_eq!(count(&a,"partial_wiring"),1,"exceptional exit must not be erased: {a}");
}
