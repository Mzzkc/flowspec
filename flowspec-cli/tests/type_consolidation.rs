//! QA-3 (QA-Surface) — Type consolidation regression tests, eprintln removal,
//! exit code stability, pipe safety, and regression tests.
//!
//! These tests verify Worker 3's Phase 1 changes (type consolidation, eprintln fix)
//! don't break observable behavior, and validate core contracts that must hold
//! through all changes.

use assert_cmd::Command;
use std::fs;
use std::path::Path;
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

// ============================================================
// 1. Type Consolidation Regression Tests (Phase 1 — CRITICAL)
// ============================================================

/// Verify lib.rs no longer defines its own Severity/Confidence enums.
/// Canonical definitions live in analyzer/diagnostic.rs.
#[test]
fn no_duplicate_type_definitions_in_lib_rs() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("flowspec/src/lib.rs");
    let content = fs::read_to_string(&lib_rs).unwrap();

    assert!(
        !content.contains("enum Severity"),
        "lib.rs still defines 'enum Severity'. Must be deleted — \
         canonical definition is in analyzer/diagnostic.rs"
    );
    assert!(
        !content.contains("enum Confidence"),
        "lib.rs still defines 'enum Confidence'. Must be deleted — \
         canonical definition is in analyzer/diagnostic.rs"
    );
}

/// Verify the aliased re-exports (as AnalyzerSeverity, as AnalyzerConfidence)
/// are replaced with direct re-exports.
#[test]
fn no_aliased_reexports() {
    let lib_rs = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("flowspec/src/lib.rs");
    let content = fs::read_to_string(&lib_rs).unwrap();

    assert!(
        !content.contains("as AnalyzerSeverity"),
        "lib.rs still aliases Severity as AnalyzerSeverity. Must re-export directly."
    );
    assert!(
        !content.contains("as AnalyzerConfidence"),
        "lib.rs still aliases Confidence as AnalyzerConfidence. Must re-export directly."
    );

    // Verify canonical re-exports exist (Severity and Confidence in a pub use block)
    assert!(
        content.contains("Severity,")
            || content.contains("Severity}")
            || content.contains("Severity }"),
        "lib.rs must re-export Severity from analyzer::diagnostic"
    );
    assert!(
        content.contains("Confidence,")
            || content.contains("Confidence}")
            || content.contains("Confidence }"),
        "lib.rs must re-export Confidence from analyzer::diagnostic"
    );
}

/// Verify --severity flag still parses all valid values after type consolidation.
#[test]
fn severity_flag_parses_all_values_after_consolidation() {
    let project = create_clean_project();

    for severity in &["critical", "warning", "info"] {
        let mut cmd = Command::cargo_bin("flowspec").unwrap();
        let output = cmd
            .args([
                "diagnose",
                project.path().to_str().unwrap(),
                "--severity",
                severity,
            ])
            .output()
            .unwrap();

        let code = output.status.code().unwrap();
        assert!(
            code == 0 || code == 2,
            "--severity {} caused exit code {} (expected 0 or 2). \
             Severity parsing may be broken after type consolidation.",
            severity,
            code
        );
    }
}

/// Verify --confidence flag still parses all valid values after type consolidation.
#[test]
fn confidence_flag_parses_all_values_after_consolidation() {
    let project = create_clean_project();

    for confidence in &["high", "moderate", "low"] {
        let mut cmd = Command::cargo_bin("flowspec").unwrap();
        let output = cmd
            .args([
                "diagnose",
                project.path().to_str().unwrap(),
                "--confidence",
                confidence,
            ])
            .output()
            .unwrap();

        let code = output.status.code().unwrap();
        assert!(
            code == 0 || code == 2,
            "--confidence {} caused exit code {} (expected 0 or 2). \
             Confidence parsing may be broken after type consolidation.",
            confidence,
            code
        );
    }
}

/// Verify severity filter ordering is preserved: info <= warning <= critical.
/// --severity info should include warnings, --severity critical should exclude them.
#[test]
fn severity_ordering_preserved_after_consolidation() {
    let project = create_project_with_issues();

    // --severity info should include warnings (info <= warning) → findings exist
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args([
        "diagnose",
        project.path().to_str().unwrap(),
        "--severity",
        "info",
    ])
    .assert()
    .code(2);

    // --severity critical should exclude warnings (critical > warning) → no findings
    let mut cmd2 = Command::cargo_bin("flowspec").unwrap();
    cmd2.args([
        "diagnose",
        project.path().to_str().unwrap(),
        "--severity",
        "critical",
    ])
    .assert()
    .code(0);
}

/// Verify from_str_checked resolves correctly after consolidation by exercising
/// the full diagnose path with --severity filtering.
#[test]
fn from_str_checked_resolves_after_consolidation() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--severity",
            "warning",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    if !stdout.trim().is_empty() {
        let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
        if let Some(diags) = parsed.as_sequence() {
            for diag in diags {
                let sev = diag["severity"].as_str().unwrap();
                assert!(
                    sev == "warning" || sev == "critical",
                    "With --severity warning, got '{}' — info diagnostics should be filtered out.",
                    sev
                );
            }
        }
    }
}

// ============================================================
// 2. eprintln Removal Verification (Phase 1)
// ============================================================

/// Verify main.rs no longer contains eprintln! calls.
#[test]
fn no_eprintln_in_main_rs() {
    let main_rs = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/main.rs");
    let content = fs::read_to_string(&main_rs).unwrap();

    assert!(
        !content.contains("eprintln!"),
        "main.rs still contains eprintln!. Must use tracing::error! instead."
    );
}

/// Verify error output still reaches stderr after switching to tracing::error!.
#[test]
fn error_output_reaches_stderr() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", "/tmp/flowspec-nonexistent-path-qa3-test"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("flowspec-nonexistent-path-qa3-test") || stderr.contains("not exist"),
        "Error message not found in stderr after eprintln removal. \
         stderr was: '{}'",
        &stderr[..stderr.len().min(500)]
    );
}

/// Verify --quiet mode still shows errors (ERROR level passes "error" filter).
#[test]
fn quiet_mode_still_shows_errors() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "analyze",
            "/tmp/flowspec-nonexistent-path-qa3-quiet",
            "--quiet",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.is_empty(),
        "stderr is empty under --quiet with an error. tracing::error! should \
         still emit at ERROR level even with the 'error' filter."
    );
}

// ============================================================
// 3. Exit Code Stability (Through All Changes)
// ============================================================

/// Verify the 0/1/2 exit code contract holds after all changes.
#[test]
fn exit_code_contract_stable() {
    // Exit 0: clean project, no findings
    let clean = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", clean.path().to_str().unwrap()])
        .assert()
        .code(0);

    // Exit 1: error (nonexistent path)
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", "/tmp/flowspec-no-such-path-qa3"])
        .assert()
        .code(1);

    // Exit 2: findings exist (diagnose on project with dead code)
    let issues = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", issues.path().to_str().unwrap()])
        .assert()
        .code(2);
}

/// Verify empty project still exits 0 with valid manifest structure.
#[test]
fn empty_project_exits_0() {
    let dir = TempDir::new().unwrap();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0), "Empty project must exit 0");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    assert!(parsed["entities"].as_sequence().unwrap().is_empty());
    assert!(parsed["diagnostics"].as_sequence().unwrap().is_empty());
}

/// Verify clap exit code 2 collision is intercepted (remapped to 1).
#[test]
fn clap_exit_code_collision_intercepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["--format", "nonexistent-format", "analyze", "."])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_ne!(
        code, 2,
        "Clap arg error produced exit 2 (collision with findings exit code). \
         Must be remapped to exit 1."
    );
    // Clap should recognize "nonexistent-format" as invalid for the Format enum
    // and return exit 1 after our interception
}

// ============================================================
// 4. Pipe Safety (After All Changes)
// ============================================================

/// Verify stdout is pure YAML with no log contamination.
#[test]
fn stdout_pure_yaml_no_logs() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();

    // Must parse as valid YAML
    let _: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("stdout is not valid YAML — pipe safety violated");

    // No log-like lines
    for line in stdout.lines() {
        let trimmed = line.trim();
        for prefix in &["TRACE", "DEBUG", "INFO", "WARN", "ERROR", "thread '", "at "] {
            assert!(
                !trimmed.starts_with(prefix),
                "stdout contains log-like line: '{}'. All logging must go to stderr.",
                line
            );
        }
    }
}

/// Verify verbose mode doesn't pollute stdout.
#[test]
fn verbose_mode_doesnt_pollute_stdout() {
    let project = create_clean_project();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap(), "--verbose"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _: serde_yaml::Value = serde_yaml::from_str(&stdout)
        .expect("stdout with --verbose is not valid YAML — verbose logging leaked to stdout");
}

// ============================================================
// 5. Regression Tests (Known Bugs)
// ============================================================

/// Verify no duplicate diagnostics (same entity+pattern+loc tuple).
#[test]
fn no_duplicate_diagnostics() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["diagnose", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();

    if let Some(diags) = parsed.as_sequence() {
        let mut seen = std::collections::HashSet::new();
        for diag in diags {
            let key = format!(
                "{}|{}|{}",
                diag["entity"].as_str().unwrap_or(""),
                diag["pattern"].as_str().unwrap_or(""),
                diag["loc"].as_str().unwrap_or("")
            );
            assert!(seen.insert(key.clone()), "Duplicate diagnostic: {}", key);
        }
    }
}

/// Verify all diagnostic IDs are unique.
#[test]
fn diagnostic_ids_unique() {
    let project = create_project_with_issues();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["diagnose", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();

    if let Some(diags) = parsed.as_sequence() {
        let mut ids = std::collections::HashSet::new();
        for diag in diags {
            let id = diag["id"].as_str().unwrap_or("");
            assert!(!id.is_empty(), "Diagnostic has empty ID");
            assert!(ids.insert(id), "Duplicate diagnostic ID: '{}'", id);
        }
    }
}

/// Verify syntax error files don't crash the pipeline.
#[test]
fn syntax_error_graceful_degradation() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("broken.py"),
        r#"
def valid():
    return 42

def broken(
    # Missing closing paren

def also_valid():
    return "works"
"#,
    )
    .unwrap();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Syntax error file should not crash (exit 1). Got exit {}.",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("Output must be valid YAML even with syntax errors");
}
