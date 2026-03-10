use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn create_project_with_issues() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        r#"
import os

def active_function():
    return 42

def dead_function():
    """This function is never called by anything."""
    return "unreachable"

def main():
    result = active_function()
    print(result)
"#,
    )
    .unwrap();
    dir
}

#[test]
fn diagnose_outputs_diagnostics_only_not_full_manifest() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["diagnose", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();

    assert!(
        parsed.is_sequence(),
        "diagnose output must be a list of diagnostics, not a manifest object.\nGot: {:?}",
        parsed
    );

    if let Some(map) = parsed.as_mapping() {
        assert!(
            !map.contains_key(&serde_yaml::Value::String("metadata".to_string())),
            "diagnose output must NOT contain 'metadata' section"
        );
    }
}

#[test]
fn diagnose_severity_filter_excludes_lower_severity() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--severity",
            "critical",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
        if let Some(diags) = parsed.as_sequence() {
            for diag in diags {
                let severity = diag["severity"].as_str().unwrap();
                assert_eq!(
                    severity, "critical",
                    "With --severity critical, got '{}' severity diagnostic.",
                    severity
                );
            }
        }
    }
}

#[test]
fn diagnose_confidence_filter_excludes_lower_confidence() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--confidence",
            "high",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
        if let Some(diags) = parsed.as_sequence() {
            for diag in diags {
                let confidence = diag["confidence"].as_str().unwrap();
                assert_eq!(
                    confidence, "high",
                    "With --confidence high, got '{}' confidence.",
                    confidence
                );
            }
        }
    }
}

#[test]
fn diagnose_checks_filter_limits_to_named_patterns() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--checks",
            "data_dead_end",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
        if let Some(diags) = parsed.as_sequence() {
            for diag in diags {
                let pattern = diag["pattern"].as_str().unwrap();
                assert_eq!(
                    pattern, "data_dead_end",
                    "With --checks data_dead_end, got '{}' pattern.",
                    pattern
                );
            }
        }
    }
}
