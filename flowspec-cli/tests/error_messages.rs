use assert_cmd::Command;

#[test]
fn io_error_includes_file_path() {
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", "/tmp/flowspec-nonexistent-project-dir-12345"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("/tmp/flowspec-nonexistent-project-dir-12345"),
        "Error message must include the failing path. Got:\n{}",
        stderr
    );
}

#[test]
fn error_messages_include_fix_suggestion() {
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", "/tmp/flowspec-nonexistent-project-dir-12345"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let has_suggestion = stderr.contains("check")
        || stderr.contains("verify")
        || stderr.contains("ensure")
        || stderr.contains("try")
        || stderr.contains("make sure")
        || stderr.contains("fix:");
    assert!(
        has_suggestion,
        "Error message lacks a fix suggestion.\nGot:\n{}",
        stderr
    );
}

#[test]
fn config_error_names_the_config_file() {
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args([
            "analyze",
            ".",
            "--config",
            "/tmp/flowspec-nonexistent-config.yaml",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("flowspec-nonexistent-config.yaml"),
        "Config error must name the config file. Got:\n{}",
        stderr
    );
}

#[test]
fn error_messages_never_contain_rust_panic_traces() {
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd.args(["analyze", "/dev/null"]).output().unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.contains("panicked at"),
        "Error output contains a Rust panic trace.\n{}",
        stderr
    );
    assert!(
        !stderr.contains("stack backtrace"),
        "Error output contains a stack backtrace.\n{}",
        stderr
    );
}

#[test]
fn error_messages_never_contain_rust_debug_output() {
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", "/tmp/flowspec-nonexistent-project-dir-12345"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.contains("Some("),
        "Error output contains raw Debug formatting.\n{}",
        stderr
    );
    assert!(
        !stderr.contains("Err("),
        "Error output contains raw Debug formatting.\n{}",
        stderr
    );
}
