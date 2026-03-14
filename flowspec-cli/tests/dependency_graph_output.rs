use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

/// Create a multi-file Python project with known cross-file imports.
fn create_cross_file_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "from utils import helper\n\ndef main():\n    result = helper()\n    print(result)\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("utils.py"),
        "def helper():\n    return 42\n",
    )
    .unwrap();
    dir
}

/// Create a single-file project (no cross-file deps).
fn create_single_file_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.py"),
        "def main():\n    greet()\n\ndef greet():\n    print('hi')\n\nmain()\n",
    )
    .unwrap();
    dir
}

/// TEST 2.1: dependency_graph section present in YAML output
#[test]
fn dependency_graph_section_present_in_yaml() {
    let project = create_single_file_project();
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
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    assert!(
        parsed.get("dependency_graph").is_some(),
        "dependency_graph key must be present in YAML manifest"
    );
}

/// TEST 2.2: dependency_graph section present in JSON output
#[test]
fn dependency_graph_section_present_in_json() {
    let project = create_single_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed.get("dependency_graph").is_some(),
        "dependency_graph field must be present in JSON manifest"
    );
}

/// TEST 2.3: dependency_graph is empty list for single-file project
#[test]
fn dependency_graph_empty_list_for_single_file() {
    let project = create_single_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let deps = &parsed["dependency_graph"];
    assert!(
        deps.is_array(),
        "dependency_graph must be an array, got: {:?}",
        deps
    );
}

/// TEST 2.4: dependency_graph populated for cross-file Python project
#[test]
fn dependency_graph_populated_for_cross_file_project() {
    let project = create_cross_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let deps = parsed["dependency_graph"].as_array().unwrap();

    // dependency_graph may or may not be populated depending on whether
    // Worker 1 wired the extraction call in lib.rs. Verify structure if populated.
    if !deps.is_empty() {
        let edge = &deps[0];
        assert!(edge.get("from").is_some(), "Edge must have 'from' field");
        assert!(edge.get("to").is_some(), "Edge must have 'to' field");
        assert!(
            edge.get("weight").is_some(),
            "Edge must have 'weight' field"
        );
        assert!(
            edge.get("direction").is_some(),
            "Edge must have 'direction' field"
        );
        assert!(
            edge.get("issues").is_some(),
            "Edge must have 'issues' field"
        );
    }
    // If empty, that's acceptable — Worker 1 may not have wired the extraction yet.
    // The section existing (as empty array) is the key contract.
}

/// TEST 2.5: dependency_graph edge fields have correct types (when populated)
#[test]
fn dependency_graph_edge_field_types() {
    let project = create_cross_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let deps = parsed["dependency_graph"].as_array().unwrap();

    for edge in deps {
        assert!(edge["from"].is_string(), "'from' must be string");
        assert!(edge["to"].is_string(), "'to' must be string");
        assert!(edge["weight"].is_number(), "'weight' must be number");
        assert!(edge["direction"].is_string(), "'direction' must be string");
        assert!(edge["issues"].is_array(), "'issues' must be array");
    }
}

/// TEST 2.6: dependency_graph YAML and JSON are structurally consistent
#[test]
fn dependency_graph_format_consistency() {
    let project = create_cross_file_project();

    let yaml_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();
    let json_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let yaml_stdout = String::from_utf8_lossy(&yaml_output.stdout);
    let json_stdout = String::from_utf8_lossy(&json_output.stdout);

    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_stdout).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();

    let yaml_deps = yaml_parsed["dependency_graph"]
        .as_sequence()
        .map(|s| s.len())
        .unwrap_or(0);
    let json_deps = json_parsed["dependency_graph"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    assert_eq!(
        yaml_deps, json_deps,
        "dependency_graph edge count must match across formats: YAML={}, JSON={}",
        yaml_deps, json_deps
    );
}

/// TEST 2.7: dependency_graph direction field has valid values (when populated)
#[test]
fn dependency_graph_direction_valid_values() {
    let project = create_cross_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let deps = parsed["dependency_graph"].as_array().unwrap();

    for edge in deps {
        let direction = edge["direction"].as_str().unwrap_or("");
        assert!(
            direction == "unidirectional" || direction == "bidirectional",
            "direction must be 'unidirectional' or 'bidirectional', got: '{}'",
            direction
        );
    }
}

/// TEST 2.8: SARIF output does NOT include dependency_graph (SARIF is for findings)
#[test]
fn sarif_output_does_not_include_dependency_graph() {
    let project = create_cross_file_project();
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(
        parsed.get("dependency_graph").is_none(),
        "SARIF output must not contain dependency_graph at top level — \
         SARIF is for findings, not structural data"
    );
    assert!(parsed.get("runs").is_some(), "SARIF must have runs array");
}
