//! Dynamic namespace operations leave absence unknown; an ordinary literal does not.
use serde_json::Value;
use std::{fs, process::Command};
#[test]
fn actual_dynamic_namespace_lookup_withholds_private_absence() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("module.py"),
        "def _helper():\n    return 1\nlookup = globals()\nlookup['_helper']()\n",
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
    assert_eq!(a["findings"]["total"], 0);
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "unresolved_reference"
            && !l["ranges"].as_array().unwrap().is_empty()));
}
