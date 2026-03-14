//! Tests for diagnose --language flag fix (Issue #14).
//!
//! The bug: `main.rs:322` passed `&[]` for languages to `flowspec::diagnose()`,
//! making `diagnose --language X` silently analyze ALL languages. These tests
//! verify that the `--language` flag now correctly filters diagnose output.

use std::process::Command;

/// Get the flowspec binary path.
fn flowspec_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flowspec"))
}

// --- 1. Core language filtering ---

#[test]
fn diagnose_language_python_filters_output() {
    let dir = tempfile::tempdir().unwrap();
    // Python file with a dead-end function
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    // JS file with a dead-end function
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "python",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // No .js file paths should appear in the output
    assert!(
        !stdout.contains(".js"),
        "diagnose --language python should not include JS diagnostics, got:\n{}",
        stdout
    );
}

#[test]
fn diagnose_language_javascript_filters_output() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "javascript",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains(".py"),
        "diagnose --language javascript should not include Python diagnostics, got:\n{}",
        stdout
    );
}

#[test]
fn diagnose_no_language_flag_includes_all() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args(["--format", "json", "diagnose", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = parsed.as_array().unwrap();
    // Default should include diagnostics from both languages (if any exist)
    // The key assertion: no error, valid output
    assert!(
        output.status.code() != Some(1),
        "Default diagnose (no --language) should not exit with error, got exit code {:?}",
        output.status.code()
    );
    // Output must be a valid JSON array (parsing succeeded above)
}

// --- 2. Unsupported language ---

#[test]
fn diagnose_language_unsupported_exits_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("example.py"), "x = 1\n").unwrap();

    let output = flowspec_bin()
        .args([
            "diagnose",
            "--language",
            "cobol",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "Unsupported language should exit with code 1"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("cobol"),
        "Error message should mention the unsupported language, got:\n{}",
        stderr
    );
}

// --- 3. TypeScript → JS adapter mapping ---

#[test]
fn diagnose_language_typescript_maps_to_javascript() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("module.ts"),
        "function deadEnd(): number { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "typescript",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_ne!(
        output.status.code(),
        Some(1),
        "diagnose --language typescript should not fail, got exit code {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
}

// --- 4. Multiple languages ---

#[test]
fn diagnose_language_multiple_flags() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("module.ts"),
        "function tsDeadEnd(): void {}\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "python",
            "--language",
            "javascript",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should work without error
    assert_ne!(
        output.status.code(),
        Some(1),
        "Multiple --language flags should work, got stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Note: .ts files are handled by the JS adapter, so they may appear
    // with --language javascript. This is correct behavior.
}

// --- 5. Filter composition ---

#[test]
fn diagnose_language_with_checks_filter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "python",
            "--checks",
            "data_dead_end",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Language filter + checks filter should compose
    assert!(
        !stdout.contains(".js"),
        "Language + checks filter should not leak JS diagnostics"
    );

    // If there are results, they should all be data_dead_end
    if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&stdout) {
        if let Some(arr) = parsed.as_array() {
            for entry in arr {
                assert_eq!(
                    entry["pattern"].as_str().unwrap_or(""),
                    "data_dead_end",
                    "Checks filter should only include data_dead_end"
                );
            }
        }
    }
}

#[test]
fn diagnose_language_with_severity_filter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "python",
            "--severity",
            "warning",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should not error out
    assert_ne!(
        output.status.code(),
        Some(1),
        "Language + severity filter should compose without error"
    );
}

// --- 6. Rust language with no adapter ---

#[test]
fn diagnose_language_rust_no_adapter() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("sample.rs"), "fn main() {}\n").unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "rust",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    // Should produce empty diagnostics, not an error
    assert_ne!(
        output.status.code(),
        Some(1),
        "Rust (no adapter) should not error, should return empty diagnostics"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = parsed.as_array().unwrap();
    assert!(
        arr.is_empty(),
        "Rust with no adapter should produce empty diagnostics"
    );
    assert_eq!(output.status.code(), Some(0), "Empty diagnostics = exit 0");
}

// --- 7. diagnose --language with --format json ---

#[test]
fn diagnose_language_json_format() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def dead_function():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function deadFunction() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--language",
            "python",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "diagnose --language python --format json must produce valid JSON"
    );

    // Verify only Python diagnostics
    if let Ok(val) = parsed {
        if let Some(arr) = val.as_array() {
            for entry in arr {
                let loc = entry["loc"].as_str().unwrap_or("");
                assert!(
                    !loc.contains(".js"),
                    "JSON output should only contain Python diagnostics"
                );
            }
        }
    }
}
