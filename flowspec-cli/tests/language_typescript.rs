use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

fn create_ts_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.ts"),
        "function greet(name: string): string {\n  return `Hello, ${name}`;\n}\n\nfunction main() {\n  greet('world');\n}\n",
    )
    .unwrap();
    // Also include a .py file to verify filtering
    fs::write(
        dir.path().join("helper.py"),
        "def helper():\n    return 42\n",
    )
    .unwrap();
    dir
}

/// TEST 3.1: --language typescript accepted by analyze
#[test]
fn analyze_language_typescript_accepted() {
    let project = create_ts_project();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--language",
            "typescript",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "--language typescript must be accepted by analyze, got exit: {}\nstderr: {}",
        code,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// TEST 3.2: --language ts accepted by analyze (abbreviated form)
#[test]
fn analyze_language_ts_accepted() {
    let project = create_ts_project();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--language",
            "ts",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        code == 0 || code == 2,
        "--language ts must be accepted, got exit: {}\nstderr: {}",
        code,
        stderr
    );
    assert!(
        !stderr.contains("unsupported language"),
        "'ts' must not be rejected as unsupported"
    );
}

/// TEST 3.3: --language ts and --language typescript produce identical results
#[test]
fn language_ts_and_typescript_produce_same_results() {
    let project = create_ts_project();

    let ts_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--language",
            "ts",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let typescript_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--language",
            "typescript",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert_eq!(
        ts_output.status.code(),
        typescript_output.status.code(),
        "ts and typescript must produce same exit code"
    );

    if ts_output.status.code() == Some(0) || ts_output.status.code() == Some(2) {
        let ts_stdout = String::from_utf8_lossy(&ts_output.stdout);
        let typescript_stdout = String::from_utf8_lossy(&typescript_output.stdout);

        let ts_parsed: serde_json::Value = serde_json::from_str(&ts_stdout).unwrap();
        let typescript_parsed: serde_json::Value =
            serde_json::from_str(&typescript_stdout).unwrap();

        let ts_entity_count = ts_parsed["metadata"]["entity_count"].as_u64();
        let typescript_entity_count = typescript_parsed["metadata"]["entity_count"].as_u64();

        assert_eq!(
            ts_entity_count, typescript_entity_count,
            "ts and typescript must produce same entity count"
        );
    }
}

/// TEST 3.4: --language typescript accepted by diagnose
#[test]
fn diagnose_language_typescript_accepted() {
    let project = create_ts_project();
    let output = flowspec()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--language",
            "typescript",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "--language typescript must be accepted by diagnose, got exit: {}",
        code
    );
}

/// TEST 3.5: --language ts accepted by diagnose
#[test]
fn diagnose_language_ts_accepted() {
    let project = create_ts_project();
    let output = flowspec()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--language",
            "ts",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "--language ts must be accepted by diagnose, got exit: {}",
        code
    );
}

/// TEST 3.6: --language typescript with trace command
#[test]
fn trace_language_typescript_accepted() {
    let project = create_ts_project();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "greet",
            "--language",
            "typescript",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unsupported language"),
        "--language typescript must not be rejected by trace"
    );
}
