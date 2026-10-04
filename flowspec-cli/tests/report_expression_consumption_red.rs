//! Value consumers in expressions resolve bindings rather than nearby spelling.
use serde_json::Value;
use std::{fs, process::Command};
fn count(body: &str) -> usize {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        format!("def _helper():\n    return 1\n{body}"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(matches!(out.status.code(), Some(0 | 3)));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    v["authority"]["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["subject"]["qualified_name"] == "module.py::_helper")
        .count()
}
#[test]
fn conditional_and_lambda_function_reads_are_consumers() {
    for body in [
        "def main():\n    return _helper if True else None\nmain()\n",
        "def main():\n    return lambda: _helper\nmain()\n",
    ] {
        assert_eq!(count(body), 0, "{body}");
    }
}
#[test]
fn lambda_parameter_and_literal_string_are_distinct_bindings() {
    assert_eq!(
        count("def main():\n    return lambda _helper: _helper\nmain()\n"),
        1
    );
    assert_eq!(count("def main():\n    return '_helper'\nmain()\n"), 1);
}
