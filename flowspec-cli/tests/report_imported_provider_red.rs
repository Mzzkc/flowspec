//! Real imported builtin providers require namespace consumer closure.
use serde_json::Value;
use std::{fs, process::Command};

fn check(prefix: &str, expression: &str, qualified: bool) {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("helpers.py"),
        "def _unused_helper():\n    return 1\n",
    )
    .unwrap();
    fs::write(
        root.path().join("main.py"),
        format!("{prefix}\ndef main():\n    return {expression}\nmain()\n"),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(root.path())
        .args(["--format", "json"])
        .output()
        .unwrap();
    let value: Value = serde_json::from_slice(&out.stdout).unwrap();
    let a = &value["authority"];
    let helper = a["findings"]["all"].as_array().unwrap().iter().any(|f| {
        f["subject"]["qualified_name"]
            .as_str()
            .is_some_and(|n| n.ends_with("::_unused_helper"))
    });
    assert_eq!(helper, qualified, "{prefix}: {a}");
    if !qualified {
        assert_eq!(out.status.code(), Some(3));
        assert!(a["losses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l["required_capabilities"]
                .as_array()
                .is_some_and(|c| c.iter().any(|v| v == "consumer_closure"))));
    } else {
        assert_eq!(out.status.code(), Some(0));
    }
}

#[test]
fn imported_builtin_lookup_is_not_private_absence() {
    check(
        "from builtins import getattr\nimport helpers",
        "getattr(helpers, '_unused_helper')",
        false,
    );
}
#[test]
fn aliased_imported_builtin_lookup_is_not_private_absence() {
    check(
        "from builtins import getattr as lookup\nimport helpers",
        "lookup(helpers, '_unused_helper')",
        false,
    );
}
#[test]
fn resolved_pure_local_provider_preserves_private_absence() {
    check(
        "import helpers\ndef getattr(value, name):\n    return 0",
        "getattr(helpers, '_unused_helper')",
        true,
    );
}
#[test]
fn literal_string_and_parameter_do_not_consume_outer_binding() {
    check(
        "def echo(_unused_helper):\n    return _unused_helper",
        "echo('_unused_helper')",
        true,
    );
}
#[test]
fn closed_private_positive_remains_qualified() {
    check("", "1", true);
}
