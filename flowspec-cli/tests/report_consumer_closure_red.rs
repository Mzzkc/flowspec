//! Direct CLI controls for Python binding reads and private absence proof.
use serde_json::Value;
use std::fs;
use std::process::Command;

fn analyze(main: &str) -> Value {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("main.py"),
        format!("def _unused_helper():\n    return 1\n{main}"),
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .args(["analyze", root.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn helper_count(report: &Value) -> usize {
    report["authority"]["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| {
            finding["pattern_id"] == "data_dead_end"
                && finding["subject"]["qualified_name"] == "main.py::_unused_helper"
        })
        .count()
}

#[test]
fn returned_function_value_consumes_binding() {
    let report = analyze("def main():\n    return _unused_helper\nmain()\n");
    assert_eq!(helper_count(&report), 0, "returned binding is consumed");
}

#[test]
fn callback_alias_consumes_binding() {
    let report = analyze("callback = _unused_helper\ncallback()\n");
    assert_eq!(
        helper_count(&report),
        0,
        "callback alias consumes the binding"
    );
}

#[test]
fn list_value_consumes_binding() {
    let report = analyze("def main():\n    return [_unused_helper]\nmain()\n");
    assert_eq!(helper_count(&report), 0, "list value consumes the binding");
}

#[test]
fn unrelated_string_and_shadow_parameter_do_not_consume_binding() {
    let report = analyze("def main(_unused_helper):\n    return '_unused_helper'\nmain(1)\n");
    assert_eq!(
        helper_count(&report),
        1,
        "same spelling is not a binding read"
    );
}

#[test]
fn local_assignment_shadows_outer_function_binding() {
    let report =
        analyze("def main():\n    _unused_helper = 42\n    return _unused_helper\nmain()\n");
    assert_eq!(
        helper_count(&report),
        1,
        "local value is a different binding"
    );
}
