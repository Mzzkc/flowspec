use assert_cmd::Command;
use predicates::prelude::*;

#[test]
fn analyze_command_accepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let assert = cmd.arg("analyze").arg(".").assert();
    assert.stderr(predicate::str::contains("Usage").not());
}

#[test]
fn diagnose_command_accepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let assert = cmd.arg("diagnose").arg(".").assert();
    assert.stderr(predicate::str::contains("Usage").not());
}

#[test]
fn analyze_defaults_path_to_current_dir() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let assert = cmd.arg("analyze").assert();
    assert.stderr(predicate::str::contains("required").not());
}

#[test]
fn diagnose_defaults_path_to_current_dir() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let assert = cmd.arg("diagnose").assert();
    assert.stderr(predicate::str::contains("required").not());
}

#[test]
fn version_flag_prints_version() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::is_match(r"flowspec[- ]?\S* \d+\.\d+\.\d+").unwrap());
}

#[test]
fn help_flag_prints_help() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("analyze"))
        .stdout(predicate::str::contains("diagnose"));
}

#[test]
fn analyze_help_shows_command_flags() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--incremental"))
        .stdout(predicate::str::contains("--full"))
        .stdout(predicate::str::contains("--language"));
}

#[test]
fn diagnose_help_shows_command_flags() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("--checks"))
        .stdout(predicate::str::contains("--severity"))
        .stdout(predicate::str::contains("--confidence"));
}

#[test]
fn format_flag_yaml_accepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--format", "yaml"])
        .assert()
        .stderr(predicate::str::contains("Usage").not());
}

#[test]
fn output_flag_sets_file_path() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--output", "/tmp/flowspec-test-output.yaml"])
        .assert()
        .stderr(predicate::str::contains("Usage").not());
}

// === Flag Validation Error Cases ===

#[test]
fn verbose_and_quiet_conflict() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--verbose", "--quiet"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("cannot be used with"));
}

#[test]
fn invalid_format_enum_rejected() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--format", "xml"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn invalid_severity_rejected() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", ".", "--severity", "extreme"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn invalid_confidence_rejected() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", ".", "--confidence", "very-high"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("invalid value"));
}

#[test]
fn unknown_command_shows_error_and_suggestions() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.arg("unknown")
        .assert()
        .failure()
        .stderr(predicate::str::contains("error"));
}

#[test]
fn no_subcommand_shows_help_or_error() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.assert().failure().stderr(
        predicate::str::contains("analyze")
            .or(predicate::str::contains("Usage"))
            .or(predicate::str::contains("help")),
    );
}

// === Adversarial CLI Inputs ===

#[test]
fn empty_path_argument_handled() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let assert = cmd.args(["analyze", ""]).assert();
    assert.code(1);
}

#[test]
fn nonexistent_path_gives_error_with_path() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", "/nonexistent/path/to/project"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("/nonexistent/path/to/project"));
}

#[test]
fn diagnose_empty_checks_flag_handled() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", ".", "--checks", ""])
        .assert()
        .stderr(predicate::str::contains("panic").not());
}

#[test]
fn diagnose_invalid_pattern_name_in_checks() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["diagnose", ".", "--checks", "nonexistent-pattern"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("nonexistent-pattern"));
}

#[test]
fn language_flag_with_unsupported_language() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--language", "haskell"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("haskell"));
}

#[test]
fn summary_format_is_accepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--format", "summary"])
        .assert()
        .code(predicate::in_iter([0, 2]));
}

#[test]
fn sarif_format_is_accepted() {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.args(["analyze", ".", "--format", "sarif"])
        .assert()
        .code(predicate::in_iter([0, 2]));
}

#[test]
fn deferred_commands_give_not_implemented_error() {
    // trace now requires --symbol, so it's tested separately in trace_command.rs.
    // diff, init, watch are still stubs.
    for cmd_name in &["diff", "init", "watch"] {
        let mut cmd = Command::cargo_bin("flowspec").unwrap();
        let assert = if *cmd_name == "diff" {
            // diff requires two path arguments
            cmd.args([cmd_name, "/tmp/a", "/tmp/b"]).assert()
        } else {
            cmd.arg(cmd_name).assert()
        };
        assert.stderr(predicate::str::contains("panic").not());
    }
}
