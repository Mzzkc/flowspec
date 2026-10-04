//! A parsed annotation is not implemented type-flow or contract evidence.
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn required_unimplemented_type_flow_is_explicit_without_erasing_closed_private_proof() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        "def _helper():\n    return 1\ndef public_api(value: int) -> int:\n    return value\n",
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
    let a = &v["authority"];
    assert!(a["losses"].as_array().unwrap().iter().any(|l| l["reason"]
        == "capability_unavailable"
        && l["required_capabilities"]
            .as_array()
            .is_some_and(|caps| caps.iter().any(|c| c == "type_flows"))
        && !l["ranges"].as_array().unwrap().is_empty()));
    let contract = a["pattern_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern_id"] == "contract_mismatch")
        .unwrap();
    assert!(contract["cells"]
        .as_array()
        .unwrap()
        .iter()
        .all(|cell| cell["state"] == "incomplete"
            && cell["reason_codes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r == "capability_unavailable")));
    assert_eq!(a["findings"]["total"],1,"a missing type-flow implementation does not itself lose the parsed function consumer universe");
}
