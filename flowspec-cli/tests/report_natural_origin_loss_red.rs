//! Ordinary production resolution losses retain the syntax operation's origin.
use serde_json::Value;
use std::{fs, process::Command};

#[test]
fn ordinary_unresolved_annotation_and_dynamic_value_have_distinct_origins() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("module.py"),
        "def api(value: Missing):\n    return getattr(value, 'field')\n",
    )
    .unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(root.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    let losses = report["authority"]["losses"].as_array().unwrap();
    for (reason, origin, line) in [
        ("unresolved_type", "Annotation", 1),
        ("unresolved_reference", "Value", 2),
    ] {
        assert!(
            losses.iter().any(|loss| loss["reason"] == reason
                && loss["origin"] == origin
                && loss["ranges"]
                    .as_array()
                    .is_some_and(|ranges| ranges.iter().any(|range| range["start_line"] == line))),
            "{reason}/{origin}: {report}"
        );
    }
}
