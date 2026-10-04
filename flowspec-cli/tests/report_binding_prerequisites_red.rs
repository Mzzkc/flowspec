//! Required binding operations are exercised on consumed source, not capability labels.
use serde_json::Value;
use std::{fs, process::Command};
fn report(source: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("helpers.py"),
        "def _helper():\n    return 1\n",
    )
    .unwrap();
    fs::write(dir.path().join("main.py"), source).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert!(matches!(output.status.code(), Some(0 | 3)));
    serde_json::from_slice(&output.stdout).unwrap()
}
fn helper_count(a: &Value) -> usize {
    a["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["subject"]["qualified_name"] == "helpers.py::_helper")
        .count()
}
#[test]
fn imported_function_values_consumed_by_return_alias_and_container() {
    for source in [
        "from helpers import _helper\ndef main():\n    return _helper\nmain()\n",
        "from helpers import _helper\ncallback = _helper\ncallback()\n",
        "from helpers import _helper\ndef main():\n    return [_helper]\nmain()\n",
    ] {
        let v = report(source);
        assert_eq!(helper_count(&v["authority"]), 0, "{source}: {v}");
    }
}
#[test]
fn unmodeled_match_prerequisite_is_ranged_debt_in_affected_cells() {
    let v = report("match 1:\n    case 1:\n        pass\n");
    let a = &v["authority"];
    assert_eq!(
        a["execution"], "incomplete",
        "unmodeled control-flow prerequisite cannot be complete"
    );
    assert_eq!(
        helper_count(a),
        0,
        "unknown potential consumers cannot qualify absence"
    );
    let loss = a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|l| l["reason"] == "unsupported_construct")
        .expect("actual match AST prerequisite loss");
    assert!(!loss["ranges"].as_array().unwrap().is_empty());
    assert!(loss["required_capabilities"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "control_flow"));
    for pattern in a["pattern_state"].as_array().unwrap() {
        if loss["affected_patterns"]
            .as_array()
            .unwrap()
            .contains(&pattern["pattern_id"])
        {
            for cell in pattern["cells"].as_array().unwrap() {
                assert_eq!(cell["state"], "incomplete");
                assert!(cell["reason_codes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|r| r == "unsupported_construct"));
            }
        }
    }
}
