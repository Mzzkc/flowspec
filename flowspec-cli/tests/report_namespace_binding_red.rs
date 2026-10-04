//! A local function with a builtin spelling is not a namespace reflection operation.
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn resolved_local_globals_binding_does_not_invent_namespace_uncertainty() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        "def _helper():\n    return 1\ndef globals():\n    return {}\nglobals()\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["authority"]["findings"]["total"], 1);
    assert!(v["authority"]["losses"].as_array().unwrap().is_empty());
}
