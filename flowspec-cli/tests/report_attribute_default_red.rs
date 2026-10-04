//! Definition-time defaults consume outer module members, unlike body parameters or strings.
use serde_json::Value;
use std::{fs, process::Command};

fn report(main: &str) -> Value {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("helpers.py"),
        "def _unused_helper():\n    return 1\n",
    )
    .unwrap();
    fs::write(dir.path().join("main.py"), main).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn helper_count(v: &Value) -> usize {
    v["authority"]["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| {
            f["pattern_id"] == "data_dead_end"
                && f["subject"]["qualified_name"] == "helpers.py::_unused_helper"
        })
        .count()
}

#[test]
fn module_attribute_defaults_consume_outer_binding_for_both_parameter_names() {
    let mut failures = Vec::new();
    for parameter in ["_unused_helper", "value"] {
        let v = report(&format!(
            "import helpers\ndef wrap({parameter}=helpers._unused_helper):\n    return 1\nwrap()\n"
        ));
        if helper_count(&v) != 0 {
            failures.push(parameter);
        }
        assert!(v["authority"]["losses"].as_array().unwrap().is_empty());
    }
    assert!(
        failures.is_empty(),
        "definition-time module-member consumers were missed: {failures:?}"
    );
}

#[test]
fn runtime_parameter_shadow_does_not_consume_outer_helper() {
    let v = report("def wrap(_unused_helper=0):\n    return _unused_helper\nwrap()\n");
    assert_eq!(helper_count(&v), 1);
    assert!(v["authority"]["losses"].as_array().unwrap().is_empty());
}

#[test]
fn literal_default_spelling_does_not_consume_outer_helper() {
    let v = report("def wrap(value='helpers._unused_helper'):\n    return value\nwrap()\n");
    assert_eq!(helper_count(&v), 1);
    assert!(v["authority"]["losses"].as_array().unwrap().is_empty());
}
