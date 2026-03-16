//! Cycle 8 QA-3 tests: focused trace output, --depth, --direction flags.
//!
//! All assertions are UNCONDITIONAL — no `if code == 0` guards.
//! Tests validate output structure, not specific flow path contents.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

/// Cross-file Python fixture with a clear call chain for tracing.
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

// ============================================================
// Category 1: Focused Trace Output (6 tests)
// ============================================================

/// T1: Trace output contains flow data — UNCONDITIONAL
#[test]
fn trace_output_contains_flow_data() {
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

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "trace --symbol main must succeed (exit 0) on fixture with main()"
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(!stdout.is_empty(), "trace output must not be empty");

    // Must contain flow-specific structure — at least one of these keys
    let has_flow_keys = stdout.contains("entry:")
        || stdout.contains("exit:")
        || stdout.contains("steps:")
        || stdout.contains("\"entry\"")
        || stdout.contains("\"steps\"");
    assert!(
        has_flow_keys,
        "trace output must contain flow-specific keys (entry/exit/steps), got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

/// T2: Trace output excludes entity list
#[test]
fn trace_output_excludes_entities() {
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

    assert_eq!(output.status.code(), Some(0), "trace must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Use line-start match to avoid false positives from "entity:" inside flow steps.
    let has_entity_section = stdout.lines().any(|line| {
        let trimmed = line.trim();
        trimmed == "entities:" || trimmed.starts_with("entities:")
    });
    assert!(
        !has_entity_section,
        "trace output must NOT contain entity list section. Got:\n{}",
        &stdout[..stdout.len().min(800)]
    );
}

/// T3: Trace output excludes diagnostics section
#[test]
fn trace_output_excludes_diagnostics() {
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

    assert_eq!(output.status.code(), Some(0), "trace must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // Check all non-flow manifest sections are absent
    let forbidden_sections = [
        "diagnostics:",
        "boundaries:",
        "dependency_graph:",
        "type_flows:",
        "metadata:",
        "summary:",
    ];
    for section in &forbidden_sections {
        let has_section = stdout.lines().any(|line| {
            let trimmed = line.trim();
            trimmed == *section || trimmed.starts_with(section)
        });
        assert!(
            !has_section,
            "trace output must NOT contain '{}' section. Got:\n{}",
            section,
            &stdout[..stdout.len().min(800)]
        );
    }
}

/// T4: Trace JSON output is valid and focused
#[test]
fn trace_json_output_is_valid_and_focused() {
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

    assert_eq!(output.status.code(), Some(0), "trace must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "trace --format json must produce valid JSON: {}\nGot:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    // Must NOT contain full-manifest sections
    let forbidden = [
        "entities",
        "diagnostics",
        "metadata",
        "summary",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    if let Some(obj) = parsed.as_object() {
        for key in &forbidden {
            assert!(
                !obj.contains_key(*key),
                "trace JSON must NOT contain '{}' key (full manifest leaking). Keys found: {:?}",
                key,
                obj.keys().collect::<Vec<_>>()
            );
        }
    }
    // If the root is an array (Vec<FlowEntry>), that's also acceptable
}

/// T5: Trace YAML output is valid and focused
#[test]
fn trace_yaml_output_is_valid_and_focused() {
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

    assert_eq!(output.status.code(), Some(0), "trace must succeed");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "trace --format yaml must produce valid YAML: {}\nGot:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    // Must NOT contain full-manifest sections at top level
    if let Some(mapping) = parsed.as_mapping() {
        let forbidden = [
            "entities",
            "diagnostics",
            "metadata",
            "summary",
            "boundaries",
            "dependency_graph",
            "type_flows",
        ];
        for key in &forbidden {
            let yaml_key = serde_yaml::Value::String(key.to_string());
            assert!(
                !mapping.contains_key(&yaml_key),
                "trace YAML must NOT contain '{}' key (full manifest leaking)",
                key
            );
        }
    }
}

/// T6: Unknown symbol exit code and error message quality
#[test]
fn trace_unknown_symbol_exit_1_helpful_error() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "nonexistent_xyz_abc",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(1), "unknown symbol must exit 1");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("nonexistent_xyz_abc"),
        "Error must echo back the symbol name for debugging. Got: {}",
        stderr
    );
    // Must provide actionable guidance
    assert!(
        stderr.contains("not found") || stderr.contains("analyze") || stderr.contains("available"),
        "Error must include guidance on what to do. Got: {}",
        stderr
    );
}

// ============================================================
// Category 2: --depth Flag (4 tests)
// ============================================================

/// T7: --depth 1 limits flow paths to 1 step
#[test]
fn trace_depth_1_limits_flow_steps() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "1",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(code, 0, "trace --depth 1 must succeed");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Must produce valid JSON: {}", e));

    // Find all "steps" arrays and verify none exceed length 1
    fn check_depth(val: &serde_json::Value, max_depth: usize) {
        match val {
            serde_json::Value::Object(map) => {
                if let Some(steps) = map.get("steps") {
                    if let Some(arr) = steps.as_array() {
                        assert!(
                            arr.len() <= max_depth,
                            "Flow path has {} steps, expected at most {}. Steps: {:?}",
                            arr.len(),
                            max_depth,
                            arr
                        );
                    }
                }
                for v in map.values() {
                    check_depth(v, max_depth);
                }
            }
            serde_json::Value::Array(arr) => {
                for v in arr {
                    check_depth(v, max_depth);
                }
            }
            _ => {}
        }
    }
    check_depth(&parsed, 1);
}

/// T8: --depth 100 allows deep traversal without error
#[test]
fn trace_depth_100_allows_deep_traversal() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "100",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "trace --depth 100 must succeed, not crash or reject"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Must produce valid JSON: {}", e));
}

/// T9: --depth 0 is handled gracefully (no crash)
#[test]
fn trace_depth_0_boundary() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "0",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "trace --depth 0 must exit 0 (empty) or 1 (error), got: {}",
        code
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "trace --depth 0 must NOT panic"
    );
}

/// T10: Default depth produces reasonable output
#[test]
fn trace_default_depth_reasonable_output() {
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
        "default depth trace must succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.is_empty(),
        "default depth must produce non-empty output"
    );

    let _parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Must produce valid JSON: {}", e));

    // Sanity check: output shouldn't be absurdly large for a 4-function fixture
    let output_len = stdout.len();
    assert!(
        output_len < 50_000,
        "Default depth trace on small fixture produced {} bytes — possible explosion",
        output_len
    );
}

// ============================================================
// Category 3: --direction Flag (4 tests)
// ============================================================

/// T11: --direction forward works (same as default)
#[test]
fn trace_direction_forward_works() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--direction",
            "forward",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "trace --direction forward must succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Must produce valid JSON: {}", e));
}

/// T12: --direction backward gives clear "not implemented" error
#[test]
fn trace_direction_backward_clear_error() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--direction",
            "backward",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 1,
        "trace --direction backward must exit 1 (not implemented)"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Must explain WHY it failed
    assert!(
        stderr.to_lowercase().contains("not")
            && (stderr.to_lowercase().contains("implemented")
                || stderr.to_lowercase().contains("supported")),
        "Error must say backward tracing is not implemented/supported. Got: {}",
        stderr
    );
    // Must suggest a fix
    assert!(
        stderr.contains("forward"),
        "Error should suggest using --direction forward. Got: {}",
        stderr
    );
}

/// T13: --direction both gives clear error (requires backward tracing)
#[test]
fn trace_direction_both_clear_error() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--direction",
            "both",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 1,
        "trace --direction both must exit 1 (not implemented)"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("panicked"),
        "trace --direction both must not panic"
    );
    // Must communicate the limitation
    assert!(
        stderr.to_lowercase().contains("not")
            && (stderr.to_lowercase().contains("implemented")
                || stderr.to_lowercase().contains("supported")),
        "Error must explain the limitation. Got: {}",
        stderr
    );
}

/// T14: Invalid --direction value produces helpful Clap error
#[test]
fn trace_direction_invalid_value_error() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--direction",
            "sideways",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(code != 0, "Invalid direction must not succeed");

    let stderr = String::from_utf8_lossy(&output.stderr);
    // Clap should list valid values
    assert!(
        stderr.contains("forward")
            || stderr.contains("invalid")
            || stderr.contains("possible values"),
        "Error should mention valid direction values. Got: {}",
        stderr
    );
}

// ============================================================
// Category 4: Flag Registration + Conditional Guard Fixes (2 tests)
// ============================================================

/// T15: --depth flag is accepted by Clap
#[test]
fn trace_depth_flag_registered() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "5",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unknown"),
        "--depth must be a registered flag, got clap error: {}",
        stderr
    );
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "--depth must not cause parser error (got exit {})",
        code
    );
}

/// T16: --direction flag is accepted by Clap
#[test]
fn trace_direction_flag_registered() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--direction",
            "forward",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unexpected argument") && !stderr.contains("unknown"),
        "--direction must be a registered flag, got clap error: {}",
        stderr
    );
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1,
        "--direction must not cause parser error (got exit {})",
        code
    );
}

// ============================================================
// Category 5: Edge Cases and Adversarial (6 tests)
// ============================================================

/// T17: Leaf symbol (no outgoing calls) produces valid output, not error
#[test]
fn trace_leaf_symbol_valid_output() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "greet",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "Leaf symbol must exit 0 (found but no flows), not 1 (not found)"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Leaf symbol trace must produce valid JSON: {}", e));
}

/// T18: --depth and --direction can be combined
#[test]
fn trace_depth_and_direction_combined() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "5",
            "--direction",
            "forward",
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "Combined --depth + --direction forward must succeed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let _parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Must produce valid JSON: {}", e));
}

/// T19: Focused trace output is pipe-safe (no logs in stdout)
#[test]
fn trace_focused_output_pipe_safe() {
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let log_patterns = ["TRACE ", "DEBUG ", "INFO ", "WARN ", "ERROR "];
    for pattern in &log_patterns {
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(pattern) || trimmed.starts_with(&format!("[{}]", pattern.trim()))
            {
                panic!(
                    "trace stdout contains log prefix '{}' in line: '{}'. Logs must go to stderr.",
                    pattern, line
                );
            }
        }
    }
}

/// T20: Trace --format sarif produces valid SARIF or clear error
#[test]
fn trace_sarif_format_valid_structure() {
    let project = create_trace_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "Trace SARIF format must exit 0 on valid fixture, got {}",
        code
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("SARIF must be valid JSON: {}", e));
    if let Some(obj) = parsed.as_object() {
        assert!(
            obj.contains_key("$schema") || obj.contains_key("version") || obj.contains_key("runs"),
            "SARIF output must contain schema, version, or runs. Keys: {:?}",
            obj.keys().collect::<Vec<_>>()
        );
    }
}

/// T21: Trace exit codes are strictly 0 or 1 (per cli.yaml)
#[test]
fn trace_exit_codes_contract() {
    // Scenario 1: valid symbol
    let project = create_trace_fixture();
    let out1 = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
        ])
        .output()
        .unwrap();
    let code1 = out1.status.code().unwrap();
    assert!(
        code1 == 0 || code1 == 1,
        "Valid symbol: exit must be 0 or 1, got {}",
        code1
    );

    // Scenario 2: invalid symbol
    let out2 = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "zzz_no_such",
        ])
        .output()
        .unwrap();
    let code2 = out2.status.code().unwrap();
    assert!(
        code2 == 0 || code2 == 1,
        "Invalid symbol: exit must be 0 or 1, got {}",
        code2
    );

    // Scenario 3: empty project
    let empty = TempDir::new().unwrap();
    let out3 = flowspec()
        .args(["trace", empty.path().to_str().unwrap(), "--symbol", "main"])
        .output()
        .unwrap();
    let code3 = out3.status.code().unwrap();
    assert!(
        code3 == 0 || code3 == 1,
        "Empty project: exit must be 0 or 1, got {}",
        code3
    );
}

/// T22: --depth with non-numeric value gives Clap error
#[test]
fn trace_depth_non_numeric_error() {
    let project = create_single_file_fixture();
    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "abc",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(code != 0, "--depth abc must not succeed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("invalid")
            || stderr.contains("parse")
            || stderr.contains("number")
            || stderr.contains("not a valid"),
        "--depth abc must produce parse error. Got: {}",
        stderr
    );
}
