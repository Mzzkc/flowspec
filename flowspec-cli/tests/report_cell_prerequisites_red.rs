//! Cell prerequisites carry the actual missing operation and consumed path.
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn actual_missing_type_flow_is_named_in_its_affected_cell() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        "def api(value: int):\n    return value\n",
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(3));
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    let p = v["authority"]["pattern_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern_id"] == "contract_mismatch")
        .unwrap();
    let cell = &p["cells"][0];
    assert!(cell["required_capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "type_flows"));
    assert!(cell["affected_paths"]
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p == "6d6f64756c652e7079"));
}
