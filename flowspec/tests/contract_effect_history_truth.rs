use flowspec::{analyze, Config};
use serde_json::Value;
fn run(source: &str, ext: &str, policy: &str) -> Value {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join(format!("source.{ext}")), source).unwrap();
    let p = d.path().join("policy.yaml");
    std::fs::write(&p, policy).unwrap();
    let c = Config::load(d.path(), Some(&p)).unwrap();
    let a = analyze(d.path(), &c, &[])
        .unwrap()
        .manifest
        .authority
        .unwrap();
    let b = flowspec::diagnose_report(d.path(), &c, &[], &flowspec::ReportOptions::default())
        .unwrap()
        .authority
        .unwrap();
    assert_eq!(a["findings"], b["findings"]);
    a
}
fn count(a: &Value, p: &str) -> usize {
    a["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["pattern_id"] == p)
        .count()
}
#[test]
fn declared_all_exit_peer_exception_requires_operation_before_throw() {
    let p="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::cleanup]\n";
    let a=run("function cleanup() { return true; } function left() { throw true; } function right() { cleanup(); return true; }","js",p);
    assert_eq!(
        count(&a, "partial_wiring"),
        1,
        "exceptional exit must not be erased: {a}"
    );
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
    assert_eq!(
        count(&a, "stale_reference"),
        1,
        "supplied identity history and actual old consumer: {a}"
    );
}
#[test]
fn resource_rivals_satisfied_irrelevant_reversed_unknown_and_exceptional() {
    let p="semantic_policy:\n  version: 1\n  resources:\n    - subjects: [source.js::left, source.js::right]\n      acquire: source.js::acquire\n      release: source.js::release\n";
    for (body, expected, unknown) in [
        ("acquire(); release(); return true;", 0, false),
        ("return true;", 0, false),
        ("release(); acquire(); return true;", 1, false),
        ("acquire(); throw true;", 1, false),
        ("acquire(); release(); throw true;", 0, false),
        ("if (false) { acquire(); } return true;", 0, false),
        ("while (true) { acquire(); } return true;", 0, true),
        ("unknown(); return true;", 0, true),
    ] {
        let source=format!("function acquire() {{ return true; }} function release() {{ return true; }} function left() {{ {body} }} function right() {{ acquire(); release(); return true; }}");
        let a = run(&source, "js", p);
        assert_eq!(count(&a, "asymmetric_handling"), expected, "{body}: {a}");
        assert_eq!(
            a["losses"]
                .as_array()
                .unwrap()
                .iter()
                .any(|l| l["reason"] == "resource_control_unresolved"),
            unknown,
            "{body}: {a}"
        );
    }
    let a=run("function acquire() { return unknown; return true; } function release() { return true; } function left() { acquire(); return true; } function right() { acquire(); release(); return true; }","js",p);
    assert_eq!(
        count(&a, "asymmetric_handling"),
        0,
        "unknown endpoint behavior must not become resource proof: {a}"
    );
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "resource_control_unresolved"));
}
#[test]
fn dynamic_annotated_argument_is_unknown_not_boolean() {
    let p="semantic_policy:\n  version: 1\n  contracts:\n    - target: source.py::accept\n      parameter_types: [boolean]\n";
    let a = run(
        "def accept(x: bool):\n    return True\ndef caller(y: str):\n    return accept(y)\n",
        "py",
        p,
    );
    assert_eq!(count(&a, "contract_mismatch"), 0);
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "contract_operand_unresolved"));
}
#[test]
fn native_literal_return_contradictions_and_satisfied_controls() {
    for (ext, bad, good) in [
        (
            "py",
            "def api() -> bool:\n    return \"bad\"\n",
            "def api() -> bool:\n    return True\n",
        ),
        (
            "ts",
            "function api(): boolean { return \"bad\"; }",
            "function api(): boolean { return true; }",
        ),
        (
            "rs",
            "fn api() -> bool { \"bad\" }",
            "fn api() -> bool { true }",
        ),
    ] {
        assert_eq!(count(&run(bad, ext, ""), "contract_mismatch"), 1, "{ext}");
        assert_eq!(count(&run(good, ext, ""), "contract_mismatch"), 0, "{ext}");
    }
}
#[test]
fn four_language_history_has_supplied_bound_shim_and_no_history_rival() {
    for (ext,source) in [
        ("py","def current():\n    return True\ndef old():\n    return current()\ndef consumer():\n    return old()\n"),
        ("js","function current() { return true; } function old() { return current(); } function consumer() { return old(); }"),
        ("ts","function current(): boolean { return true; } function old(): boolean { return current(); } function consumer(): boolean { return old(); }"),
        ("rs","fn current() -> bool { true } fn old() -> bool { current() } fn consumer() -> bool { old() }")
    ] {
        let p=format!("semantic_policy:\n  version: 1\n  history:\n    - event_id: supplied-v1-to-v2\n      old: source.{ext}::old\n      new: source.{ext}::current\n      state: shim\n");
        assert_eq!(count(&run(source,ext,&p),"stale_reference"),1,"{ext}");
        assert_eq!(count(&run(source,ext,""),"stale_reference"),0,"history cannot be inferred {ext}");
        assert_eq!(count(&run(&source.replace("return old()","return current()").replace("{ old() }","{ current() }"),ext,&p),"stale_reference"),0,"satisfied history {ext}");
    }
}
#[test]
fn removed_history_and_missing_file_are_precise_unknowns() {
    let p="semantic_policy:\n  version: 1\n  history:\n    - event_id: supplied-removal\n      old: gone.js::old\n      state: removed\n";
    let a = run("function consumer() { return old(); }", "js", p);
    assert_eq!(count(&a, "stale_reference"), 0);
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "history_old_binding_unresolved"));
    assert_eq!(
        count(
            &run("function consumer() { return old(); }", "js", ""),
            "stale_reference"
        ),
        0
    );
}
#[test]
fn same_name_unrelated_file_is_not_contract_or_history_authority() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("source.js"),"function accept(x) { return true; } function current() { return true; } function old() { return current(); }").unwrap();
    std::fs::write(d.path().join("other.js"),"function accept(x) { return true; } function old() { return true; } function consumer() { accept(\"text\"); return old(); }").unwrap();
    let p = d.path().join("policy.yaml");
    std::fs::write(&p,"semantic_policy:\n  version: 1\n  contracts:\n    - target: source.js::accept\n      parameter_types: [boolean]\n  history:\n    - event_id: supplied-v1-to-v2\n      old: source.js::old\n      new: source.js::current\n      state: shim\n").unwrap();
    let c = Config::load(d.path(), Some(&p)).unwrap();
    let a = analyze(d.path(), &c, &[])
        .unwrap()
        .manifest
        .authority
        .unwrap();
    assert_eq!(
        count(&a, "contract_mismatch"),
        0,
        "wrong bound declaration: {a}"
    );
    assert_eq!(count(&a, "stale_reference"), 0, "wrong bound old name: {a}");
}
#[test]
fn byte_literals_are_not_invented_string_operands() {
    for (ext, source) in [
        (
            "py",
            "def accept(x: str):\n    return True\ndef caller():\n    return accept(b\"bytes\")\n",
        ),
        (
            "rs",
            "fn accept(x: &str) -> bool { true } fn caller() -> bool { accept(b\"bytes\") }",
        ),
    ] {
        let p=format!("semantic_policy:\n  version: 1\n  contracts:\n    - target: source.{ext}::accept\n      parameter_types: [boolean]\n");
        let a = run(source, ext, &p);
        let uses = a["findings"]["all"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|f| {
                f["pattern_id"] == "contract_mismatch"
                    && f["cause"]["parts"]["relation"]
                        == "bound-literal-argument-type-contradiction"
            })
            .count();
        assert_eq!(uses, 0, "unsupported byte literal is not a string: {a}");
        assert!(a["losses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["reason"] == "contract_operand_unresolved"));
    }
}
#[test]
fn python_shadowed_primitive_annotation_is_not_builtin_type_proof() {
    let a = run(
        "class bool:\n    pass\ndef api() -> bool:\n    return \"text\"\n",
        "py",
        "",
    );
    assert_eq!(
        count(&a, "contract_mismatch"),
        0,
        "bool denotes the local class: {a}"
    );
}
#[test]
fn all_exit_peer_unknown_callee_return_cannot_hide_exception_before_cleanup() {
    let p="semantic_policy:\n  version: 1\n  peers:\n    - subjects: [source.js::left, source.js::right]\n      required_operations: [source.js::cleanup]\n";
    let a=run("function cleanup() { return true; } function dangerous() { return missing; } function left() { dangerous(); cleanup(); return true; } function right() { cleanup(); return true; }","js",p);
    assert_eq!(count(&a, "partial_wiring"), 0);
    assert!(
        a["losses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["reason"] == "policy_control_unresolved"),
        "unknown callee behavior must retain all-exit debt: {a}"
    );
}
#[test]
fn four_language_wrong_file_binding_and_unknown_resource_rivals() {
    for (ext,source,other) in [
        ("py","def accept(x: bool):\n    return True\ndef current():\n    return True\ndef old():\n    return current()\n","def accept(x: str):\n    return True\ndef old():\n    return True\ndef consumer():\n    accept(\"text\")\n    return old()\n"),
        ("js","function accept(x) { return true; } function current() { return true; } function old() { return current(); }","function accept(x) { return true; } function old() { return true; } function consumer() { accept(\"text\"); return old(); }"),
        ("ts","function accept(x: boolean): boolean { return true; } function current(): boolean { return true; } function old(): boolean { return current(); }","function accept(x: string): boolean { return true; } function old(): boolean { return true; } function consumer(): boolean { accept(\"text\"); return old(); }"),
        ("rs","fn accept(x: bool) -> bool { true } fn current() -> bool { true } fn old() -> bool { current() }","fn accept(x: &str) -> bool { true } fn old() -> bool { true } fn consumer() -> bool { accept(\"text\"); old() }")
    ] {
        let d=tempfile::tempdir().unwrap();
        std::fs::write(d.path().join(format!("source.{ext}")),source).unwrap();std::fs::write(d.path().join(format!("other.{ext}")),other).unwrap();
        let p=d.path().join("policy.yaml");std::fs::write(&p,format!("semantic_policy:\n  version: 1\n  contracts:\n    - target: source.{ext}::accept\n      parameter_types: [boolean]\n  history:\n    - event_id: supplied-v1-to-v2\n      old: source.{ext}::old\n      new: source.{ext}::current\n      state: shim\n")).unwrap();
        let c=Config::load(d.path(),Some(&p)).unwrap();let a=analyze(d.path(),&c,&[]).unwrap().manifest.authority.unwrap();
        assert_eq!(count(&a,"contract_mismatch"),0,"same name different bound file {ext}: {a}");assert_eq!(count(&a,"stale_reference"),0,"same name different bound file {ext}: {a}");
        let p=format!("semantic_policy:\n  version: 1\n  resources:\n    - subjects: [source.{ext}::old, source.{ext}::current]\n      acquire: source.{ext}::missing\n      release: source.{ext}::accept\n");
        let a=run(source,ext,&p);assert_eq!(count(&a,"asymmetric_handling"),0);assert!(a["losses"].as_array().unwrap().iter().any(|l|l["reason"]=="resource_binding_unresolved"));
    }
}
#[test]
fn literal_exceptional_resource_exit_is_supported_in_three_languages_and_rust_unwind_is_held() {
    for (ext,source,supported) in [
        ("py","def acquire():\n    return True\ndef release():\n    return True\ndef left():\n    acquire()\n    raise True\ndef right():\n    acquire()\n    release()\n    return True\n",true),
        ("js","function acquire() { return true; } function release() { return true; } function left() { acquire(); throw true; } function right() { acquire(); release(); return true; }",true),
        ("ts","function acquire(): boolean { return true; } function release(): boolean { return true; } function left(): boolean { acquire(); throw true; } function right(): boolean { acquire(); release(); return true; }",true),
        ("rs","fn acquire() -> bool { true } fn release() -> bool { true } fn left() -> bool { acquire(); panic!(\"stop\"); } fn right() -> bool { acquire(); release(); return true; }",false)
    ] {
        let p=format!("semantic_policy:\n  version: 1\n  resources:\n    - subjects: [source.{ext}::left, source.{ext}::right]\n      acquire: source.{ext}::acquire\n      release: source.{ext}::release\n");
        let a=run(source,ext,&p);assert_eq!(count(&a,"asymmetric_handling"),usize::from(supported),"{ext}: {a}");
        if !supported {assert!(a["losses"].as_array().unwrap().iter().any(|l|l["reason"]=="resource_control_unresolved"));}
    }
}
