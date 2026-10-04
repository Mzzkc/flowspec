//! Default expressions execute in the defining scope, before parameter bindings.
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn same_spelled_parameter_default_is_a_real_outer_function_consumer() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        "def _helper():\n    return 1\ndef public_api(_helper=_helper):\n    return _helper\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(
        v["authority"]["findings"]["all"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| !f["subject"]["qualified_name"]
                .as_str()
                .is_some_and(|n| n.ends_with("::_helper"))),
        "a real default-value escape cannot be asserted unused"
    );
    assert!(v["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| !f["entity"]
            .as_str()
            .is_some_and(|n| n.ends_with("::_helper"))));
}
