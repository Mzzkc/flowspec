use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn create_clean_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

def main():
    result = greet("world")
    print(result)

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();
    dir
}

fn create_project_with_issues() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        r#"
import os

def used_function():
    return 42

def dead_function():
    """This function is never called by anything."""
    return "unreachable"

def main():
    result = used_function()
    print(result)
"#,
    )
    .unwrap();
    dir
}

// === Category 1: JSON Output Validity (P0) ===

#[test]
fn json_output_is_valid_json() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.success() || output.status.code() == Some(2),
        "analyze --format json failed with exit code {:?}. stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout is not valid JSON: {:?}\nOutput was:\n{}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn json_manifest_has_all_eight_required_sections() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Not valid JSON: {}. Output: {}",
            e,
            &stdout[..stdout.len().min(300)]
        )
    });

    let required_sections = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];

    let obj = parsed.as_object().expect("JSON root must be an object");
    for section in &required_sections {
        assert!(
            obj.contains_key(*section),
            "Missing required manifest section '{}' in JSON output. Present keys: {:?}",
            section,
            obj.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn json_empty_sections_are_empty_arrays() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    for section in &["flows", "boundaries", "type_flows"] {
        let val = &parsed[section];
        assert!(
            val.is_array(),
            "Section '{}' must be a JSON array, got: {:?}",
            section,
            val
        );
    }
}

#[test]
fn diagnose_json_output_is_valid_json() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert!(
        output.status.code() == Some(0) || output.status.code() == Some(2),
        "diagnose --format json returned unexpected exit code {:?}. stderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "diagnose stdout is not valid JSON: {:?}\nOutput: {}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );

    let val = parsed.unwrap();
    assert!(
        val.is_array(),
        "diagnose --format json must produce a JSON array, got: {:?}",
        val
    );
}

// === Category 2: JSON/YAML Equivalence (P0) ===

#[test]
fn json_yaml_same_sections() {
    let project = create_project_with_issues();

    let yaml_output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();
    let yaml_stdout = String::from_utf8(yaml_output.stdout).unwrap();
    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_stdout).unwrap();

    let json_output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8(json_output.stdout).unwrap();
    let json_val: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();

    let mut yaml_keys: Vec<String> = yaml_val
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    let mut json_keys: Vec<String> = json_val.as_object().unwrap().keys().cloned().collect();

    yaml_keys.sort();
    json_keys.sort();

    assert_eq!(
        yaml_keys, json_keys,
        "YAML sections {:?} != JSON sections {:?}",
        yaml_keys, json_keys
    );
}

#[test]
fn json_yaml_entity_count_matches() {
    let project = create_project_with_issues();

    let yaml_output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();
    let yaml_val: serde_yaml::Value =
        serde_yaml::from_str(&String::from_utf8(yaml_output.stdout).unwrap()).unwrap();

    let json_output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let json_val: serde_json::Value =
        serde_json::from_str(&String::from_utf8(json_output.stdout).unwrap()).unwrap();

    let yaml_entities = yaml_val["entities"].as_sequence().unwrap().len();
    let json_entities = json_val["entities"].as_array().unwrap().len();

    assert_eq!(
        yaml_entities, json_entities,
        "Entity count mismatch: YAML={}, JSON={}",
        yaml_entities, json_entities
    );
}

#[test]
fn json_entity_uses_abbreviated_field_names() {
    let project = create_project_with_issues();
    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let parsed: serde_json::Value =
        serde_json::from_str(&String::from_utf8(output.stdout).unwrap()).unwrap();

    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be a JSON array");
    assert!(!entities.is_empty(), "need entities for this test");

    let first = &entities[0];
    for field in &["id", "kind", "vis", "sig", "loc"] {
        assert!(
            first.get(field).is_some(),
            "Entity missing abbreviated field '{}' in JSON. Keys: {:?}",
            field,
            first.as_object().map(|o| o.keys().collect::<Vec<_>>())
        );
    }

    for bad_field in &["visibility", "signature", "location"] {
        assert!(
            first.get(bad_field).is_none(),
            "Entity uses non-abbreviated field name '{}' in JSON.",
            bad_field
        );
    }
}

// === Category 3: CLI Integration (P1) ===

#[test]
fn json_output_flag_writes_valid_json_to_file() {
    let project = create_clean_project();
    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
            "--output",
            output_path,
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "stdout should be empty when --output is used with JSON. Got: {}",
        &stdout[..stdout.len().min(200)]
    );

    let file_content = std::fs::read_to_string(output_path).unwrap();
    assert!(!file_content.is_empty(), "--output file is empty");
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&file_content);
    assert!(
        parsed.is_ok(),
        "--output file is not valid JSON: {:?}",
        parsed.err()
    );
}

#[test]
fn sarif_format_is_now_implemented() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--format", "sarif"])
        .assert()
        .code(predicate::in_iter([0, 2]));
}

#[test]
fn json_format_exit_code_0_on_clean_project() {
    let project = create_clean_project();
    Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .code(0);
}

#[test]
fn json_format_exit_code_2_on_findings() {
    let project = create_project_with_issues();
    Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .code(2);
}

#[test]
fn json_stdout_contains_only_json() {
    let project = create_clean_project();
    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();

    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout contains non-JSON content. Pipe safety violated.\nFirst 300 chars: {}",
        &stdout[..stdout.len().min(300)]
    );

    let log_patterns = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    for pattern in &log_patterns {
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(pattern) || trimmed.starts_with(&format!("[{}]", pattern)) {
                panic!(
                    "stdout contains log-like line: '{}'. Logs must go to stderr only.",
                    line
                );
            }
        }
    }
}

#[test]
fn json_format_quiet_no_stderr() {
    let project = create_clean_project();
    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.is_empty(),
        "--quiet --format json should produce no stderr. Got:\n{}",
        stderr
    );
}

#[test]
fn json_manifest_within_10x_size_constraint() {
    let project = create_project_with_issues();
    let source_size: u64 = walkdir::WalkDir::new(project.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "py").unwrap_or(false))
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let manifest_size = output.stdout.len() as u64;
    let effective_source = source_size.max(1024);
    assert!(
        manifest_size <= effective_source * 10,
        "JSON manifest ({} bytes) exceeds 10x source ({} bytes). Ratio: {:.1}x",
        manifest_size,
        source_size,
        manifest_size as f64 / source_size as f64
    );
}
