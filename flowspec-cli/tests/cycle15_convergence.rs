//! Cycle 15 QA-3 convergence tests — post-commit CLI stability verification.
//!
//! 22 tests across 6 categories: format validity on Rust fixtures, exit code
//! contract stability, pipe safety regression, cross-format consistency,
//! filter flag stability, and prior-cycle regression guards.

use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// Helpers
// ============================================================================

fn flowspec() -> Command {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.current_dir(workspace_root());
    cmd
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn rust_fixture_path() -> String {
    workspace_root()
        .join("tests/fixtures/rust")
        .to_str()
        .unwrap()
        .to_string()
}

fn create_clean_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        "def greet(name):\n    return f'Hello, {name}'\n\n\
         def main():\n    result = greet('world')\n    print(result)\n\n\
         if __name__ == '__main__':\n    main()\n",
    )
    .unwrap();
    dir
}

fn create_python_with_dead_code() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        "import os\n\n\
         def active_function():\n    return 42\n\n\
         def dead_function():\n    return 'unreachable'\n\n\
         def another_dead():\n    return 0\n\n\
         def main():\n    result = active_function()\n    print(result)\n",
    )
    .unwrap();
    dir
}

fn create_mixed_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.py"),
        "def greet():\n    return 'hi'\n\ndef main():\n    greet()\n\nmain()\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function greet() { return 'hi'; }\ngreet();\n",
    )
    .unwrap();
    dir
}

fn create_minimal_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("tiny.py"), "def f():\n    pass\n\nf()\n").unwrap();
    dir
}

// ============================================================================
// Category 1: Post-Commit Format Validity (T1-T7)
// ============================================================================

#[test]
fn yaml_valid_after_phase1_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec().args(["analyze", &fixture]).output().unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "Rust fixture YAML output is not valid: {:?}\nOutput:\n{}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn json_valid_after_phase1_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Rust fixture JSON output invalid: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let sections = [
        "metadata",
        "summary",
        "entities",
        "flows",
        "boundaries",
        "diagnostics",
        "dependency_graph",
        "type_flows",
    ];
    for section in &sections {
        assert!(
            parsed.get(section).is_some(),
            "JSON missing required section '{}' for Rust fixture",
            section
        );
    }
}

#[test]
fn sarif_valid_after_phase1_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["--format", "sarif", "analyze", &fixture])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Rust fixture SARIF output invalid: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let schema = parsed["$schema"].as_str().unwrap_or("");
    assert!(
        schema.contains("sarif"),
        "SARIF $schema must contain 'sarif', got: {}",
        schema
    );
    assert_eq!(
        parsed["version"].as_str().unwrap_or(""),
        "2.1.0",
        "SARIF version must be 2.1.0"
    );
    let runs = parsed["runs"].as_array();
    assert!(runs.is_some(), "SARIF must have a 'runs' array");
    assert!(
        !runs.unwrap().is_empty(),
        "SARIF 'runs' array must be non-empty"
    );
}

#[test]
fn summary_valid_after_phase1_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "summary"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.is_empty(),
        "Summary output must not be empty for Rust fixture"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unreachable") && !stderr.contains("panicked"),
        "Summary format caused panic/unreachable. stderr:\n{}",
        &stderr[..stderr.len().min(500)]
    );
}

#[test]
fn diagnose_yaml_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec().args(["diagnose", &fixture]).output().unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "Rust fixture diagnose YAML invalid: {:?}\nOutput:\n{}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn diagnose_json_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["--format", "json", "diagnose", &fixture])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Rust fixture diagnose JSON invalid: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    assert!(
        parsed.is_array(),
        "Diagnose JSON output must be an array, got: {}",
        parsed
    );

    if let Some(arr) = parsed.as_array() {
        for entry in arr {
            assert!(
                entry.get("pattern").is_some(),
                "Diagnose entry missing 'pattern'"
            );
            assert!(
                entry.get("severity").is_some(),
                "Diagnose entry missing 'severity'"
            );
            assert!(
                entry.get("entity").is_some(),
                "Diagnose entry missing 'entity'"
            );
        }
    }
}

#[test]
fn diagnose_sarif_rust_fixture() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["--format", "sarif", "diagnose", &fixture])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "Rust fixture diagnose SARIF invalid: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    assert_eq!(
        parsed["version"].as_str().unwrap_or(""),
        "2.1.0",
        "SARIF version must be 2.1.0"
    );
    assert!(
        parsed.get("runs").is_some(),
        "SARIF diagnose output must have 'runs'"
    );
}

// ============================================================================
// Category 2: Exit Code Contract Stability (T8-T10)
// ============================================================================

#[test]
fn exit_code_0_clean_all_formats() {
    let project = create_clean_python();
    for format in &["yaml", "json", "sarif", "summary"] {
        let output = flowspec()
            .args([
                "analyze",
                project.path().to_str().unwrap(),
                "--format",
                format,
            ])
            .output()
            .unwrap();

        assert_eq!(
            output.status.code().unwrap(),
            0,
            "Clean project with format '{}' should exit 0, got {:?}",
            format,
            output.status.code()
        );
    }
}

#[test]
fn exit_code_2_diagnose_findings_all_formats() {
    let project = create_python_with_dead_code();
    for format in &["yaml", "json", "sarif"] {
        let output = flowspec()
            .args([
                "--format",
                format,
                "diagnose",
                project.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();

        assert_eq!(
            output.status.code().unwrap(),
            2,
            "Diagnose with findings for format '{}' should exit 2, got {:?}",
            format,
            output.status.code()
        );
    }
}

#[test]
fn exit_code_1_error_preserved() {
    let output = flowspec()
        .args(["analyze", "/tmp/flowspec-c15-no-such-path"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Nonexistent path should exit 1"
    );
}

// ============================================================================
// Category 3: Pipe Safety Regression (T11-T13)
// ============================================================================

#[test]
fn yaml_pipe_safe_no_logs() {
    let fixture = rust_fixture_path();
    let output = flowspec().args(["analyze", &fixture]).output().unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();

    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "YAML stdout is contaminated — not valid YAML"
    );

    let log_prefixes = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    for line in stdout.lines() {
        let trimmed = line.trim();
        for prefix in &log_prefixes {
            assert!(
                !trimmed.starts_with(prefix) && !trimmed.starts_with(&format!("[{}]", prefix)),
                "stdout contains log line: '{}'. Pipe safety violated.",
                line
            );
        }
    }
}

#[test]
fn json_pipe_safe_pure() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["--format", "json", "analyze", &fixture])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "JSON stdout is not pure JSON. Pipe safety violated.\nFirst 300 chars: {}",
        &stdout[..stdout.len().min(300)]
    );
}

#[test]
fn sarif_pipe_safe_pure() {
    let project = create_python_with_dead_code();
    let output = flowspec()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "SARIF stdout is not pure JSON. Pipe safety violated.\nFirst 300 chars: {}",
        &stdout[..stdout.len().min(300)]
    );
}

// ============================================================================
// Category 4: Cross-Format Consistency (T14-T15)
// ============================================================================

#[test]
fn entity_count_yaml_json_match_rust_fixture() {
    let fixture = rust_fixture_path();

    let yaml_output = flowspec().args(["analyze", &fixture]).output().unwrap();
    let yaml_stdout = String::from_utf8(yaml_output.stdout).unwrap();
    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_stdout).unwrap();
    let yaml_entities = yaml_parsed["entities"]
        .as_sequence()
        .map(|s| s.len())
        .unwrap_or(0);

    let json_output = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8(json_output.stdout).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();
    let json_entities = json_parsed["entities"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    assert_eq!(
        yaml_entities, json_entities,
        "Entity count mismatch: YAML={} vs JSON={} for Rust fixture",
        yaml_entities, json_entities
    );
}

#[test]
fn diagnostic_count_yaml_json_match() {
    let project = create_python_with_dead_code();

    let yaml_output = flowspec()
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();
    let yaml_stdout = String::from_utf8(yaml_output.stdout).unwrap();
    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_stdout).unwrap();
    let yaml_diags = yaml_parsed["diagnostics"]
        .as_sequence()
        .map(|s| s.len())
        .unwrap_or(0);

    let json_output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8(json_output.stdout).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();
    let json_diags = json_parsed["diagnostics"]
        .as_array()
        .map(|a| a.len())
        .unwrap_or(0);

    assert_eq!(
        yaml_diags, json_diags,
        "Diagnostic count mismatch: YAML={} vs JSON={} for Python dead code project",
        yaml_diags, json_diags
    );
}

// ============================================================================
// Category 5: Filter Flag Stability (T16-T18)
// ============================================================================

#[test]
fn diagnose_checks_filter_stable() {
    let project = create_python_with_dead_code();
    let output = flowspec()
        .args([
            "--format",
            "json",
            "diagnose",
            "--checks",
            "data_dead_end",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = parsed.as_array().expect("Diagnose JSON should be an array");

    for entry in entries {
        let pattern = entry["pattern"].as_str().unwrap_or("");
        assert_eq!(
            pattern, "data_dead_end",
            "--checks data_dead_end should filter to only data_dead_end, got: {}",
            pattern
        );
    }
}

#[test]
fn diagnose_severity_filter_stable() {
    let project = create_python_with_dead_code();
    let output = flowspec()
        .args([
            "--format",
            "json",
            "diagnose",
            "--severity",
            "warning",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = parsed.as_array().expect("Diagnose JSON should be an array");

    let valid_severities = ["warning", "critical"];
    for entry in entries {
        let severity = entry["severity"].as_str().unwrap_or("");
        assert!(
            valid_severities.contains(&severity),
            "--severity warning should only show warning and above, got: {}",
            severity
        );
    }
}

#[test]
fn analyze_language_filter_stable() {
    let project = create_mixed_project();
    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    if let Some(entities) = parsed["entities"].as_array() {
        let has_js = entities.iter().any(|e| {
            e["loc"]
                .as_str()
                .map(|l| l.contains(".js"))
                .unwrap_or(false)
        });
        assert!(
            !has_js,
            "JS entities leaked through --language python filter"
        );
    }
}

// ============================================================================
// Category 6: Regression Guards (T19-T22)
// ============================================================================

#[test]
fn manifest_8_sections_preserved() {
    let project = create_clean_python();
    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let mapping = parsed.as_mapping().expect("YAML root must be a mapping");

    let required = [
        "metadata",
        "summary",
        "entities",
        "flows",
        "boundaries",
        "diagnostics",
        "dependency_graph",
        "type_flows",
    ];
    for section in &required {
        assert!(
            mapping.contains_key(&serde_yaml::Value::String(section.to_string())),
            "Manifest missing required section '{}'. Present keys: {:?}",
            section,
            mapping
                .keys()
                .filter_map(|k| k.as_str())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn byte_floor_still_active() {
    let project = create_minimal_python();
    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Minimal project should succeed, got exit {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.is_empty(),
        "Byte floor failed — minimal project produced empty output"
    );

    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "Byte floor active but output is not valid YAML for minimal project"
    );
}

#[test]
fn no_unreachable_any_format() {
    let project = create_clean_python();
    for format in &["yaml", "json", "sarif", "summary"] {
        let output = flowspec()
            .args([
                "analyze",
                project.path().to_str().unwrap(),
                "--format",
                format,
            ])
            .output()
            .unwrap();

        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("unreachable") && !stderr.contains("panicked"),
            "Format '{}' caused panic/unreachable. stderr:\n{}",
            format,
            &stderr[..stderr.len().min(500)]
        );
    }
}

#[test]
fn confidence_field_present() {
    let project = create_python_with_dead_code();
    let output = flowspec()
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

    let valid_confidence = ["high", "moderate", "low"];
    if let Some(diagnostics) = parsed["diagnostics"].as_array() {
        assert!(
            !diagnostics.is_empty(),
            "Dead code project should produce diagnostics"
        );
        for diag in diagnostics {
            let conf = diag["confidence"].as_str().unwrap_or_else(|| {
                panic!(
                    "Diagnostic missing 'confidence' field: {}",
                    serde_json::to_string_pretty(diag).unwrap()
                )
            });
            assert!(
                valid_confidence.contains(&conf),
                "Invalid confidence value '{}', expected one of {:?}",
                conf,
                valid_confidence
            );
        }
    }
}
