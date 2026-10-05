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
#[test]
fn four_language_literal_arguments_contradict_exact_declared_schema() {
    for (ext,source) in [
        ("py","def accept(x: bool):\n    return True\ndef caller():\n    return accept(\"bad\")\n"),
        ("js","function accept(x) { return true; } function caller() { return accept(\"bad\"); }"),
        ("ts","function accept(x: boolean): boolean { return true; } function caller(): boolean { return accept(\"bad\"); }"),
        ("rs","fn accept(x: bool) -> bool { true } fn caller() -> bool { accept(\"bad\") }")
    ] {
        let p=format!("semantic_policy:\n  version: 1\n  contracts:\n    - target: source.{ext}::accept\n      parameter_types: [boolean]\n");
        let a=run(source,ext,&p);
        assert_eq!(count(&a,"contract_mismatch"),1,"literal bound schema contradiction {ext}: {a}");
        assert_eq!(count(&run(&source.replace("\"bad\"",if ext=="py" {"True"} else {"true"}),ext,&p),"contract_mismatch"),0);
    }
}
#[test]
fn four_language_resource_acquire_release_obligation_tracks_exit_order() {
    for (ext,source) in [
        ("py","def acquire():\n    return True\ndef release():\n    return True\ndef left():\n    acquire()\n    return True\ndef right():\n    acquire()\n    release()\n    return True\n"),
        ("js","function acquire() { return true; } function release() { return true; } function left() { acquire(); return true; } function right() { acquire(); release(); return true; }"),
        ("ts","function acquire(): boolean { return true; } function release(): boolean { return true; } function left(): boolean { acquire(); return true; } function right(): boolean { acquire(); release(); return true; }"),
        ("rs","fn acquire() -> bool { true } fn release() -> bool { true } fn left() -> bool { acquire(); return true; } fn right() -> bool { acquire(); release(); return true; }")
    ] {
        let p=format!("semantic_policy:\n  version: 1\n  resources:\n    - subjects: [source.{ext}::left, source.{ext}::right]\n      acquire: source.{ext}::acquire\n      release: source.{ext}::release\n");
        let a=run(source,ext,&p);
        assert_eq!(count(&a,"asymmetric_handling"),1,"resource leak {ext}: {a}");
    }
}
#[test]
fn supplied_shim_identity_history_targets_only_bound_old_calls() {
    let p="semantic_policy:\n  version: 1\n  history:\n    - event_id: supplied-v1-to-v2\n      old: source.js::old\n      new: source.js::current\n      state: shim\n";
    let a=run("function current() { return true; } function old() { return current(); } function consumer() { return old(); }","js",p);
    assert_eq!(count(&a,"stale_reference"),1,"supplied identity history and actual old consumer: {a}");
}
