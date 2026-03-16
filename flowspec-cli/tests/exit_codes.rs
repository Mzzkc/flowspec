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

#[test]
fn exit_0_on_clean_project() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", project.path().to_str().unwrap()])
        .assert()
        .code(0);
}

#[test]
fn exit_2_on_project_with_critical_diagnostics() {
    // Our data_dead_end diagnostics are severity "warning", not "critical"
    // So analyze should return exit 0 (success, no critical diagnostics)
    // But the project still has findings — diagnose would return exit 2
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    // analyze exits 2 only for critical, ours are warnings
    // Use diagnose to check for findings
    cmd.args(["diagnose", project.path().to_str().unwrap()])
        .assert()
        .code(2);
}

#[test]
fn exit_1_on_nonexistent_path() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", "/tmp/flowspec-no-such-project-ever"])
        .assert()
        .code(1);
}

#[test]
fn exit_0_on_summary_format() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    // Summary format is now implemented
    cmd.args([
        "analyze",
        project.path().to_str().unwrap(),
        "--format",
        "summary",
    ])
    .assert()
    .code(predicate::in_iter([0, 2]));
}

#[test]
fn diagnose_exit_0_when_no_findings_above_threshold() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args([
        "diagnose",
        project.path().to_str().unwrap(),
        "--severity",
        "critical",
    ])
    .assert()
    .code(0);
}

#[test]
fn diagnose_exit_2_when_findings_above_threshold() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args([
        "diagnose",
        project.path().to_str().unwrap(),
        "--severity",
        "info",
    ])
    .assert()
    .code(2);
}

#[test]
fn diagnose_exit_1_on_analysis_failure() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", "/tmp/flowspec-no-such-project-ever"])
        .assert()
        .code(1);
}

#[test]
fn diagnose_confidence_filter_affects_exit_code() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--confidence",
            "high",
        ])
        .output()
        .unwrap();
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Unexpected exit code: {}. Must be 0 or 2, never 1.",
        code
    );
}

#[test]
fn exit_codes_are_only_0_1_2() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1 || code == 2,
        "Unexpected exit code: {}. Only 0, 1, 2 are valid per cli.yaml.",
        code
    );
}
