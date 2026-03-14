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
    assert_eq!(
        map.len(),
        8,
        "Manifest must have exactly 8 sections, got: {}",
        map.len()
    );
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
    assert_eq!(
        obj.len(),
        8,
        "JSON manifest must have exactly 8 fields, got: {}",
        obj.len()
    );
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
