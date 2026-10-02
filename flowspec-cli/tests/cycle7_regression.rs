use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

fn create_standard_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        "def greet(name: str) -> str:\n    return f'Hello, {name}'\n\n\
         def main():\n    result = greet('world')\n    print(result)\n\n\
         if __name__ == '__main__':\n    main()\n",
    )
    .unwrap();
    dir
}

fn create_project_with_issues() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        "def used_function():\n    return 42\n\n\
         def dead_function():\n    return 'unreachable'\n\n\
         def main():\n    result = used_function()\n    print(result)\n",
    )
    .unwrap();
    dir
}

/// TEST 4.1: analyze YAML output still has 8 manifest sections
#[test]
fn regression_yaml_manifest_has_8_sections() {
    let project = create_standard_project();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("YAML must still parse after Cycle 7 changes");
    let map = parsed.as_mapping().unwrap();
    assert_yaml_report_root_shape(map);
}

/// TEST 4.2: analyze JSON output still has 8 manifest sections
#[test]
fn regression_json_manifest_has_8_sections() {
    let project = create_standard_project();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("JSON must still parse after Cycle 7 changes");
    let obj = parsed.as_object().unwrap();
    assert_json_report_root_shape(obj);
}

/// TEST 4.3: SARIF output still validates after Cycle 7 changes
#[test]
fn regression_sarif_still_valid() {
    let project = create_project_with_issues();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("SARIF must still be valid JSON after Cycle 7 changes");

    assert!(
        parsed["$schema"].as_str().unwrap_or("").contains("sarif"),
        "SARIF $schema must still reference sarif spec"
    );
    assert_eq!(parsed["version"], "2.1.0");
    assert!(parsed["runs"].is_array());
}

/// TEST 4.4: diagnose exit codes unchanged
#[test]
fn regression_diagnose_exit_codes_unchanged() {
    let project = create_project_with_issues();
    let output = flowspec()
        .args(["diagnose", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "diagnose with findings must still exit 2"
    );
}

/// TEST 4.5: analyze --language python still filters correctly
#[test]
fn regression_analyze_language_python_filters() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def py_func():\n    return 42\n").unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function jsFunc() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--language",
            "python",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    for entity in entities {
        let loc = entity["loc"].as_str().unwrap_or("");
        assert!(
            !loc.contains(".js"),
            "analyze --language python must not include JS entities: {}",
            loc
        );
    }
}

/// TEST 4.6: diagnose --checks filter still works
#[test]
fn regression_diagnose_checks_filter() {
    let project = create_project_with_issues();
    let output = flowspec()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--checks",
            "data_dead_end",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = parsed.as_array().unwrap();

    for entry in arr {
        assert_eq!(
            entry["pattern"].as_str().unwrap(),
            "data_dead_end",
            "--checks filter must still work after Cycle 7 changes"
        );
    }
}

// REPORT_ROOT_SHAPE_AMENDMENT_BEGIN
fn assert_yaml_report_root_shape(map: &serde_yaml::Mapping) {
    for section in [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ] {
        assert!(
            map.contains_key(serde_yaml::Value::String(section.to_owned())),
            "missing top-level payload section: {section}"
        );
    }
    assert_eq!(
        map.get(serde_yaml::Value::String("schema_version".to_owned()))
            .and_then(serde_yaml::Value::as_u64),
        Some(2),
        "schema_version must be numeric 2 at the root"
    );
    for field in ["execution", "outcome"] {
        assert!(
            map.get(serde_yaml::Value::String(field.to_owned()))
                .and_then(serde_yaml::Value::as_str)
                .is_some(),
            "{field} must be a root string"
        );
    }
    assert!(
        map.get(serde_yaml::Value::String("findings_present".to_owned()))
            .and_then(serde_yaml::Value::as_bool)
            .is_some(),
        "findings_present must be a root boolean"
    );
    assert!(
        map.get(serde_yaml::Value::String("completeness".to_owned()))
            .and_then(serde_yaml::Value::as_mapping)
            .is_some(),
        "completeness must be a root mapping"
    );
}

fn assert_json_report_root_shape(obj: &serde_json::Map<String, serde_json::Value>) {
    for section in [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ] {
        assert!(
            obj.contains_key(section),
            "missing top-level payload section: {section}"
        );
    }
    assert_eq!(
        obj.get("schema_version")
            .and_then(serde_json::Value::as_u64),
        Some(2),
        "schema_version must be numeric 2 at the root"
    );
    for field in ["execution", "outcome"] {
        assert!(
            obj.get(field).and_then(serde_json::Value::as_str).is_some(),
            "{field} must be a root string"
        );
    }
    assert!(
        obj.get("findings_present")
            .and_then(serde_json::Value::as_bool)
            .is_some(),
        "findings_present must be a root boolean"
    );
    assert!(
        obj.get("completeness")
            .and_then(serde_json::Value::as_object)
            .is_some(),
        "completeness must be a root object"
    );
}

#[test]
fn yaml_report_root_shape_negative_controls() {
    use serde_yaml::{Mapping, Value};
    let key = |name: &str| Value::String(name.to_owned());
    let payload = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    let state = [
        "schema_version",
        "execution",
        "findings_present",
        "outcome",
        "completeness",
    ];
    let mut root = Mapping::new();
    for name in payload {
        root.insert(key(name), Value::Mapping(Mapping::new()));
    }
    root.insert(key("schema_version"), serde_yaml::to_value(2).unwrap());
    root.insert(key("execution"), key("complete"));
    root.insert(key("findings_present"), Value::Bool(false));
    root.insert(key("outcome"), key("clean"));
    root.insert(key("completeness"), Value::Mapping(Mapping::new()));
    assert_yaml_report_root_shape(&root);
    let rejects = |candidate: &Mapping| {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_yaml_report_root_shape(candidate)
            }))
            .is_err(),
            "invalid YAML root passed shape check"
        );
    };
    for name in payload {
        let mut candidate = root.clone();
        candidate.remove(key(name));
        candidate.insert(key("renamed_payload"), Value::Null);
        rejects(&candidate);
    }
    for name in state {
        let mut candidate = root.clone();
        candidate.remove(key(name));
        rejects(&candidate);
        for parent in ["metadata", "summary"] {
            let mut nested = root.clone();
            let value = nested.remove(key(name)).unwrap();
            nested
                .get_mut(key(parent))
                .unwrap()
                .as_mapping_mut()
                .unwrap()
                .insert(key(name), value);
            rejects(&nested);
        }
    }
    for (name, value) in [
        ("schema_version", serde_yaml::to_value(1).unwrap()),
        ("schema_version", key("2")),
        ("findings_present", key("false")),
        ("findings_present", Value::Null),
        ("execution", Value::Null),
        ("outcome", Value::Null),
        ("completeness", Value::Sequence(vec![])),
        ("completeness", Value::Null),
    ] {
        let mut candidate = root.clone();
        candidate.insert(key(name), value);
        rejects(&candidate);
    }
    root.insert(key("extra_benign_key"), Value::Null);
    assert_yaml_report_root_shape(&root);
}

#[test]
fn json_report_root_shape_negative_controls() {
    use serde_json::{Map, Value};
    let payload = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    let state = [
        "schema_version",
        "execution",
        "findings_present",
        "outcome",
        "completeness",
    ];
    let mut root = Map::new();
    for name in payload {
        root.insert(name.to_owned(), Value::Object(Map::new()));
    }
    root.insert("schema_version".to_owned(), serde_json::json!(2));
    root.insert("execution".to_owned(), serde_json::json!("complete"));
    root.insert("findings_present".to_owned(), Value::Bool(false));
    root.insert("outcome".to_owned(), serde_json::json!("clean"));
    root.insert("completeness".to_owned(), Value::Object(Map::new()));
    assert_json_report_root_shape(&root);
    let rejects = |candidate: &Map<String, Value>| {
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                assert_json_report_root_shape(candidate)
            }))
            .is_err(),
            "invalid JSON root passed shape check"
        );
    };
    for name in payload {
        let mut candidate = root.clone();
        candidate.remove(name);
        candidate.insert("renamed_payload".to_owned(), Value::Null);
        rejects(&candidate);
    }
    for name in state {
        let mut candidate = root.clone();
        candidate.remove(name);
        rejects(&candidate);
        for parent in ["metadata", "summary"] {
            let mut nested = root.clone();
            let value = nested.remove(name).unwrap();
            nested
                .get_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .insert(name.to_owned(), value);
            rejects(&nested);
        }
    }
    for (name, value) in [
        ("schema_version", serde_json::json!(1)),
        ("schema_version", serde_json::json!("2")),
        ("findings_present", serde_json::json!("false")),
        ("findings_present", Value::Null),
        ("execution", Value::Null),
        ("outcome", Value::Null),
        ("completeness", Value::Array(vec![])),
        ("completeness", Value::Null),
    ] {
        let mut candidate = root.clone();
        candidate.insert(name.to_owned(), value);
        rejects(&candidate);
    }
    root.insert("extra_benign_key".to_owned(), Value::Null);
    assert_json_report_root_shape(&root);
}
// REPORT_ROOT_SHAPE_AMENDMENT_END
