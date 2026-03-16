use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

/// Create a Python fixture with a clear call chain for tracing.
fn create_trace_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.py"),
        r#"
from helpers import process

def main():
    result = process("input")
    print(result)

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("helpers.py"),
        r#"
def process(data):
    cleaned = sanitize(data)
    return transform(cleaned)

def sanitize(text):
    return text.strip()

def transform(text):
    return text.upper()
"#,
    )
    .unwrap();
    dir
}

fn create_single_file_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.py"),
        "def main():\n    greet()\n\ndef greet():\n    print('hi')\n\nmain()\n",
    )
    .unwrap();
    dir
}

/// TEST 1.1: --symbol flag is registered and accepted by clap
#[test]
fn trace_symbol_flag_accepted() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "--symbol must be a registered flag, got clap error: {}",
        stderr
    );
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "trace --symbol must exit 0 or 1, got: {}",
        code
    );
}

/// TEST 1.2: --symbol short form -s also works
#[test]
fn trace_short_symbol_flag_accepted() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args(["trace", project.path().to_str().unwrap(), "-s", "main"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument"),
        "-s must be accepted as short form of --symbol"
    );
}

/// TEST 1.3: --symbol without value produces clap error, not crash
#[test]
fn trace_symbol_without_value_gives_clap_error() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args(["trace", project.path().to_str().unwrap(), "--symbol"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code(), Some(1));
    assert!(
        !stderr.contains("panicked"),
        "--symbol without value must not panic"
    );
    assert!(
        stderr.contains("require") || stderr.contains("value") || stderr.contains("error"),
        "Should indicate a missing value, got stderr: {}",
        stderr
    );
}

/// TEST 1.4: Unknown symbol exits 1 with helpful error message
#[test]
fn trace_unknown_symbol_exits_1_with_message() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "nonexistent_symbol_xyz",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "Unknown symbol must exit 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nonexistent_symbol_xyz")
            || stderr.contains("not found")
            || stderr.contains("not yet implemented"),
        "Error must mention the symbol name or explain unavailability: {}",
        stderr
    );
}

/// TEST 1.5: trace with --format yaml produces valid YAML
#[test]
fn trace_format_yaml_produces_valid_yaml() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "trace --symbol main must succeed on valid fixture"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "trace --format yaml must produce valid YAML, got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

/// TEST 1.6: trace with --format json produces valid JSON
#[test]
fn trace_format_json_produces_valid_json() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "trace --symbol main must succeed on valid fixture"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "trace --format json must produce valid JSON, got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

/// TEST 1.7: trace with --format summary succeeds (not unreachable!() panic)
#[test]
fn trace_format_summary_no_unreachable_panic() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code(),
        Some(0),
        "summary format must succeed now that SummaryFormatter is implemented"
    );
    assert!(
        !stderr.contains("panicked"),
        "format summary must NOT cause unreachable!() panic"
    );
    assert!(
        !stderr.contains("unreachable"),
        "Must not hit unreachable!() code path"
    );
}

/// TEST 1.8: trace stdout is pipe-safe (no log leakage)
#[test]
fn trace_stdout_pipe_safe() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let log_patterns = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    for pattern in &log_patterns {
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(pattern) || trimmed.starts_with(&format!("[{}]", pattern)) {
                panic!(
                    "trace stdout contains log-like line: '{}'. Logs must go to stderr.",
                    line
                );
            }
        }
    }
}

/// TEST 1.9: trace on empty project exits 1 gracefully
#[test]
fn trace_empty_project_exits_1() {
    let dir = TempDir::new().unwrap();
    let output = flowspec()
        .args(["trace", dir.path().to_str().unwrap(), "--symbol", "main"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(code, 1, "Empty project should exit 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!stderr.contains("panicked"), "Empty project must not panic");
}

/// TEST 1.10: trace combined with --language python
#[test]
fn trace_with_language_flag() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def main():\n    return 42\n").unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function main() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec()
        .args([
            "trace",
            dir.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "trace --language must not crash, got exit: {}",
        code
    );
}

/// TEST 1.11: trace without --symbol gives clap error about required flag
#[test]
fn trace_without_symbol_mentions_required_flag() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args(["trace", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--symbol") || stderr.contains("-s"),
        "Error should mention the missing --symbol flag: {}",
        stderr
    );
}

/// TEST 1.12: trace --symbol with qualified name (module::name format)
#[test]
fn trace_symbol_qualified_name() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "helpers::process",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "Qualified symbol names must not crash: exit {}",
        code
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "Qualified symbol name must not panic"
    );
}

/// TEST 1.13: trace on nonexistent path exits 1
#[test]
fn trace_nonexistent_path_exits_1() {
    let output = flowspec()
        .args([
            "trace",
            "/tmp/flowspec-no-such-trace-path",
            "--symbol",
            "main",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("/tmp/flowspec-no-such-trace-path")
            || stderr.contains("not exist")
            || stderr.contains("not found"),
        "Error should mention the invalid path: {}",
        stderr
    );
}

/// TEST 1.14: trace exit codes are only 0 or 1
#[test]
fn trace_exit_codes_are_only_0_or_1() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "trace exit code must be 0 or 1 (per cli.yaml), got: {}",
        code
    );
}
