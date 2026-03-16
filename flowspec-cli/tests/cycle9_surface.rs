//! QA-3 Cycle 9 tests — SymbolNotFound fix, CommandNotImplemented suggestion,
//! summary formatter integration, cross-format consistency.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

fn create_trace_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.py"),
        r#"
from helpers import process
def main():
    data = process("input")
    return data
"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("helpers.py"),
        r#"
def process(data):
    cleaned = sanitize(data)
    return transform(cleaned)
def sanitize(data):
    return data.strip()
def transform(data):
    return data.upper()
"#,
    )
    .unwrap();
    dir
}

fn create_python_fixture() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.py"),
        r#"
def main():
    result = process("hello")
    return result

def process(data):
    return data.upper()
"#,
    )
    .unwrap();
    dir
}

// ==========================================================================
// T1: Backward direction error does NOT say "symbol not found:"
// ==========================================================================
#[test]
fn backward_direction_error_uses_command_not_implemented() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "process",
            "--direction",
            "backward",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap();

    // C11: backward tracing is now implemented — exit 0
    assert_eq!(
        code, 0,
        "Backward direction must exit 0 (now implemented), got {}",
        code
    );
}

// ==========================================================================
// T2: Both direction error does NOT say "symbol not found:"
// ==========================================================================
#[test]
fn both_direction_error_uses_command_not_implemented() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "process",
            "--direction",
            "both",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap();

    // C11: both tracing is now implemented — exit 0
    assert_eq!(
        code, 0,
        "Both direction must exit 0 (now implemented), got {}",
        code
    );
}

// ==========================================================================
// T3: CommandNotImplemented errors include actionable suggestion text
// ==========================================================================
#[test]
fn command_not_implemented_includes_suggestion() {
    for (cmd_name, args) in &[
        ("diff", vec!["diff", "/tmp/a", "/tmp/b"]),
        ("init", vec!["init"]),
        ("watch", vec!["watch"]),
    ] {
        let output = flowspec().args(args).output().unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);

        // UNCONDITIONAL: must contain "not yet implemented"
        assert!(
            stderr.contains("not yet implemented"),
            "{} command must say 'not yet implemented'.\nGot:\n{}",
            cmd_name,
            stderr
        );

        // UNCONDITIONAL: must contain a suggestion/fix hint
        let has_suggestion = stderr.contains("fix:")
            || stderr.contains("planned")
            || stderr.contains("use ")
            || stderr.contains("try ");
        assert!(
            has_suggestion,
            "{} command error must include actionable suggestion text.\nGot:\n{}",
            cmd_name, stderr
        );
    }
}

// ==========================================================================
// T4: Backward/Both direction now work (C11 trace refactor)
// ==========================================================================
#[test]
fn direction_errors_suggest_forward() {
    // C11: backward and both tracing are now implemented. Verify they succeed.
    for direction in &["backward", "both"] {
        let project = create_trace_fixture();
        let output = flowspec()
            .args([
                "trace",
                project.path().to_str().unwrap(),
                "--symbol",
                "process",
                "--direction",
                direction,
            ])
            .output()
            .unwrap();

        let code = output.status.code().unwrap();
        assert_eq!(
            code, 0,
            "--direction {} must succeed (exit 0). Got: {}",
            direction, code
        );
    }
}

// ==========================================================================
// T16: --format summary produces output, not FormatNotImplemented
// ==========================================================================
#[test]
fn analyze_format_summary_produces_output() {
    let project = create_python_fixture();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    // UNCONDITIONAL: must NOT return FormatNotImplemented
    assert!(
        !stderr.contains("not yet implemented"),
        "--format summary must be implemented, not return FormatNotImplemented.\nStderr:\n{}",
        stderr
    );

    // UNCONDITIONAL: exit 0 or 2 (valid run), not 1 (error)
    assert!(
        code == 0 || code == 2,
        "--format summary must exit 0 or 2 on valid project, got {}.\nStderr:\n{}",
        code,
        stderr
    );

    // UNCONDITIONAL: stdout must have content
    assert!(
        !stdout.trim().is_empty(),
        "--format summary must produce output on stdout"
    );
}

// ==========================================================================
// T17: Summary output contains expected sections
// ==========================================================================
#[test]
fn summary_contains_expected_sections() {
    let project = create_python_fixture();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    let has_files = stdout.contains("file") || stdout.contains("File");
    let has_entities =
        stdout.contains("entit") || stdout.contains("symbol") || stdout.contains("Symbol");
    let has_diagnostics = stdout.contains("diagnostic")
        || stdout.contains("Diagnostic")
        || stdout.contains("finding")
        || stdout.contains("Finding");

    assert!(has_files, "Summary must mention files.\nGot:\n{}", stdout);
    assert!(
        has_entities,
        "Summary must mention entities/symbols.\nGot:\n{}",
        stdout
    );
    assert!(
        has_diagnostics,
        "Summary must mention diagnostics/findings.\nGot:\n{}",
        stdout
    );
}

// ==========================================================================
// T18: Summary output stays within ~2K token budget
// ==========================================================================
#[test]
fn summary_output_size_bounded() {
    let project = create_python_fixture();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // ~2K tokens ≈ ~8KB text. Allow generous upper bound of 16KB.
    assert!(
        stdout.len() < 16_384,
        "Summary output must be bounded (~2K tokens). Got {} bytes, which exceeds 16KB limit.",
        stdout.len()
    );

    // Minimum sanity: must have some content (at least 50 chars)
    assert!(
        stdout.len() > 50,
        "Summary output suspiciously small: {} bytes",
        stdout.len()
    );
}

// ==========================================================================
// T19: Summary for diagnose command also works
// ==========================================================================
#[test]
fn diagnose_format_summary_produces_output() {
    let project = create_python_fixture();
    let output = flowspec()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap();

    assert!(
        !stderr.contains("not yet implemented"),
        "Diagnose --format summary must not return FormatNotImplemented.\n{}",
        stderr
    );
    assert!(
        code == 0 || code == 2,
        "Diagnose --format summary must exit 0 or 2, got {}.\n{}",
        code,
        stderr
    );
}

// ==========================================================================
// T20: Summary for trace command also works
// ==========================================================================
#[test]
fn trace_format_summary_produces_output() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "process",
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    let code = output.status.code().unwrap();

    assert!(
        !stderr.contains("not yet implemented"),
        "Trace --format summary must not return FormatNotImplemented.\n{}",
        stderr
    );
    // Trace exits 0 on success, 1 on error
    assert!(
        code == 0 || code == 1,
        "Trace --format summary must exit 0 or 1, got {}.\n{}",
        code,
        stderr
    );
}

// ==========================================================================
// T21: Empty project summary doesn't crash
// ==========================================================================
#[test]
fn summary_empty_project_no_crash() {
    let empty_dir = TempDir::new().unwrap();
    let output = flowspec()
        .args([
            "analyze",
            empty_dir.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panic"),
        "Empty project must not panic with --format summary.\n{}",
        stderr
    );
}

// ==========================================================================
// T22: Summary diagnostic count matches JSON count
// ==========================================================================
#[test]
fn summary_diagnostic_count_consistent_with_json() {
    let project = create_python_fixture();

    // Get JSON output for ground truth
    let json_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8_lossy(&json_output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();
    let json_diag_count = parsed["diagnostics"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    // Get summary output
    let summary_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "summary",
        ])
        .output()
        .unwrap();
    let summary_stdout = String::from_utf8_lossy(&summary_output.stdout);

    // Summary must mention the same count somewhere
    let count_str = json_diag_count.to_string();
    if json_diag_count > 0 {
        assert!(
            summary_stdout.contains(&count_str),
            "Summary must reflect the correct diagnostic count ({}).\nSummary:\n{}",
            json_diag_count,
            summary_stdout
        );
    }
}
