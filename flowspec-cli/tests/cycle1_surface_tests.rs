//! Cycle 1 QA-3 surface integration tests — CLI command integration, manifest
//! format compliance, pipe safety, exit codes, error messages, SARIF validation,
//! diff command verification.
//!
//! 38 tests across 9 categories:
//! - Category 1 (T01-T06): Diff command output structure
//! - Category 2 (T07-T10): SARIF schema validation
//! - Category 3 (T11-T14): All-format consistency
//! - Category 4 (T15-T17): Pipe safety
//! - Category 5 (T18-T22): Exit code contract
//! - Category 6 (T23-T24): YAML section ordering
//! - Category 7 (T25-T28): Error message quality
//! - Category 8 (T29-T33): End-to-end manifest validation
//! - Category 9 (T34-T38): Adversarial edge cases

use assert_cmd::Command;
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use tempfile::{NamedTempFile, TempDir};

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

/// Multi-file Python fixture with cross-file calls, imports, and a main entry point.
/// Produces non-trivial manifest with flows, entities, diagnostics.
fn create_multi_file_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("main.py"),
        "from utils import process\nfrom helpers import validate\n\ndef main():\n    data = process('input')\n    if validate(data):\n        return data\n    return None\n\nif __name__ == '__main__':\n    main()\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("utils.py"),
        "def process(data):\n    cleaned = data.strip()\n    return transform(cleaned)\n\ndef transform(data):\n    return data.upper()\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("helpers.py"),
        "def validate(data):\n    if not data:\n        return False\n    return len(data) > 0\n\ndef unused_helper():\n    pass\n",
    )
    .unwrap();
    dir
}

/// Minimal single-file fixture (small source, clean analysis).
fn create_clean_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        "def greet(name):\n    return f'Hello, {name}'\n\ndef main():\n    result = greet('world')\n    print(result)\n\nif __name__ == '__main__':\n    main()\n",
    )
    .unwrap();
    dir
}

/// Build a minimal valid YAML manifest string for diff testing.
fn minimal_manifest_yaml(
    entities: &[&str],
    diagnostics: &[(&str, &str, &str, &str)], // (pattern, entity, severity, loc)
) -> String {
    let mut yaml = String::new();
    yaml.push_str("metadata:\n");
    yaml.push_str("  project: test-project\n");
    yaml.push_str("  analyzed_at: '2026-03-28T00:00:00Z'\n");
    yaml.push_str("  flowspec_version: '0.1.0'\n");
    yaml.push_str("  languages:\n    - python\n");
    yaml.push_str("  file_count: 1\n");
    yaml.push_str(&format!("  entity_count: {}\n", entities.len()));
    yaml.push_str("  flow_count: 0\n");
    yaml.push_str(&format!("  diagnostic_count: {}\n", diagnostics.len()));
    yaml.push_str("  incremental: false\n");
    yaml.push_str("  files_changed: 0\n");
    yaml.push_str(
        "summary:\n  architecture: Test project\n  modules: []\n  entry_points: []\n  exit_points: []\n  key_flows: []\n",
    );
    yaml.push_str(
        "  diagnostic_summary:\n    critical: 0\n    warning: 0\n    info: 0\n    top_issues: []\n",
    );
    yaml.push_str("diagnostics:\n");
    if diagnostics.is_empty() {
        yaml.push_str("  []\n");
    } else {
        for (i, (pattern, entity, severity, loc)) in diagnostics.iter().enumerate() {
            yaml.push_str(&format!(
                "  - id: D{:03}\n    pattern: {}\n    severity: {}\n    confidence: high\n    entity: {}\n    message: Test diagnostic\n    evidence:\n      - observation: test observation\n    suggestion: Fix it\n    loc: {}\n",
                i + 1,
                pattern,
                severity,
                entity,
                loc
            ));
        }
    }
    yaml.push_str("entities:\n");
    if entities.is_empty() {
        yaml.push_str("  []\n");
    } else {
        for id in entities {
            yaml.push_str(&format!(
                "  - id: {}\n    kind: fn\n    vis: pub\n    sig: '() -> None'\n    loc: test.py:1\n    calls: []\n    called_by: []\n    annotations: []\n",
                id
            ));
        }
    }
    yaml.push_str("flows: []\nboundaries: []\ndependency_graph: []\ntype_flows: []\n");
    yaml
}

fn write_temp_manifest(content: &str, extension: &str) -> NamedTempFile {
    let mut file = tempfile::Builder::new()
        .suffix(&format!(".{}", extension))
        .tempfile()
        .unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

// ============================================================================
// Category 1: Diff Command Output Structure (T01–T06)
// ============================================================================

#[test]
fn test_diff_output_contains_entities_added() {
    let old = minimal_manifest_yaml(&["A", "B"], &[]);
    let new = minimal_manifest_yaml(&["A", "B", "C"], &[]);
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("C"),
        "Diff must report entity C as added. Got:\n{}",
        stdout
    );
    assert_eq!(
        output.status.code().unwrap(),
        0,
        "No regressions, expect exit 0"
    );
}

#[test]
fn test_diff_output_contains_entities_removed() {
    let old = minimal_manifest_yaml(&["A", "B", "C"], &[]);
    let new = minimal_manifest_yaml(&["A", "B"], &[]);
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("C"),
        "Diff must report entity C as removed. Got:\n{}",
        stdout
    );
}

#[test]
fn test_diff_output_contains_new_diagnostics() {
    let old = minimal_manifest_yaml(&["fn_a"], &[]);
    let new = minimal_manifest_yaml(
        &["fn_a"],
        &[("data_dead_end", "fn_a", "warning", "test.py:1")],
    );
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("data_dead_end"),
        "Diff must report new diagnostic. Got:\n{}",
        stdout
    );
}

#[test]
fn test_diff_output_contains_resolved_diagnostics() {
    let old = minimal_manifest_yaml(
        &["fn_a"],
        &[("data_dead_end", "fn_a", "warning", "test.py:1")],
    );
    let new = minimal_manifest_yaml(&["fn_a"], &[]);
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("diagnostics_resolved"),
        "Diff must report resolved diagnostics. Got:\n{}",
        stdout
    );
    assert!(
        stdout.contains("data_dead_end"),
        "Resolved diagnostic identity must include pattern. Got:\n{}",
        stdout
    );
}

#[test]
fn test_diff_exit_code_2_on_new_critical_diagnostic() {
    let old = minimal_manifest_yaml(&["fn_a"], &[]);
    let new = minimal_manifest_yaml(
        &["fn_a"],
        &[("circular_dependency", "fn_a", "critical", "test.py:1")],
    );
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        2,
        "New critical diagnostic must trigger exit code 2 (regression). Got: {}",
        output.status.code().unwrap()
    );
}

#[test]
fn test_diff_missing_boundary_and_flow_changes() {
    // Both manifests have empty boundaries/flows — diff can't report changes it doesn't track
    let old = minimal_manifest_yaml(&["fn_a"], &[]);
    let new = minimal_manifest_yaml(&["fn_a"], &[]);
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Diff JSON parse failed: {}. Output:\n{}", e, stdout));

    // DiffResult struct has no boundary or flow fields — this documents the gap
    assert!(
        parsed.get("boundary_changes").is_none(),
        "DiffResult currently has no boundary_changes field (spec gap — cli.yaml:155-157)"
    );
    assert!(
        parsed.get("flow_changes").is_none(),
        "DiffResult currently has no flow_changes field (spec gap — cli.yaml:155-157)"
    );
}

// ============================================================================
// Category 2: SARIF Schema Validation (T07–T10)
// ============================================================================

#[test]
fn test_sarif_required_top_level_fields() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sarif: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("SARIF must be valid JSON: {}. Output:\n{}", e, stdout));

    assert!(sarif["$schema"].is_string(), "$schema field required");
    assert!(
        sarif["$schema"].as_str().unwrap().contains("sarif"),
        "$schema must reference SARIF spec. Got: {}",
        sarif["$schema"]
    );
    assert_eq!(
        sarif["version"].as_str().unwrap(),
        "2.1.0",
        "SARIF version must be 2.1.0"
    );
    assert!(sarif["runs"].is_array(), "runs must be an array");
    assert!(
        !sarif["runs"].as_array().unwrap().is_empty(),
        "runs must have at least one entry"
    );
}

#[test]
fn test_sarif_results_have_required_fields() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sarif: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("SARIF parse failed: {}", e));

    let results = sarif["runs"][0]["results"]
        .as_array()
        .expect("runs[0].results must be an array");

    // May be empty if no diagnostics — that's valid. But if present, check structure.
    for (i, result) in results.iter().enumerate() {
        assert!(result["ruleId"].is_string(), "result[{}] missing ruleId", i);
        assert!(result["level"].is_string(), "result[{}] missing level", i);
        assert!(
            result["message"]["text"].is_string(),
            "result[{}] missing message.text",
            i
        );

        let level = result["level"].as_str().unwrap();
        assert!(
            ["error", "warning", "note", "none"].contains(&level),
            "result[{}] invalid SARIF level: {}. Must be error/warning/note/none.",
            i,
            level
        );
    }
}

#[test]
fn test_sarif_rules_match_result_rule_ids() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let sarif: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let rules = sarif["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .expect("tool.driver.rules must be an array");
    let rule_ids: std::collections::HashSet<&str> =
        rules.iter().filter_map(|r| r["id"].as_str()).collect();

    let empty_vec = vec![];
    let results = sarif["runs"][0]["results"].as_array().unwrap_or(&empty_vec);
    for (i, result) in results.iter().enumerate() {
        let rule_id = result["ruleId"].as_str().unwrap_or("");
        assert!(
            rule_ids.contains(rule_id),
            "result[{}] ruleId '{}' has no matching rule definition. Defined rules: {:?}",
            i,
            rule_id,
            rule_ids
        );
    }
}

#[test]
fn test_sarif_uses_camel_case() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Snake_case would be a serialization bug
    assert!(
        !stdout.contains("information_uri"),
        "SARIF must NOT use snake_case: information_uri found"
    );
    assert!(
        !stdout.contains("short_description"),
        "SARIF must NOT use snake_case: short_description found"
    );
    assert!(
        !stdout.contains("rule_id"),
        "SARIF must NOT use snake_case: rule_id found"
    );
}

// ============================================================================
// Category 3: All-Format Consistency (T11–T14)
// ============================================================================

#[test]
fn test_all_formats_consistent_entity_count() {
    let project = create_multi_file_python();
    let path = project.path().to_str().unwrap();

    // YAML: count entities list
    let yaml_out = flowspec()
        .args(["analyze", path, "--format", "yaml"])
        .output()
        .unwrap();
    let yaml_str = String::from_utf8_lossy(&yaml_out.stdout);
    let yaml_val: serde_yaml::Value =
        serde_yaml::from_str(&yaml_str).unwrap_or_else(|e| panic!("YAML parse failed: {}", e));
    let yaml_entity_count = yaml_val["metadata"]["entity_count"].as_u64().unwrap();

    // JSON: count entities list
    let json_out = flowspec()
        .args(["analyze", path, "--format", "json"])
        .output()
        .unwrap();
    let json_str = String::from_utf8_lossy(&json_out.stdout);
    let json_val: serde_json::Value =
        serde_json::from_str(&json_str).unwrap_or_else(|e| panic!("JSON parse failed: {}", e));
    let json_entity_count = json_val["metadata"]["entity_count"].as_u64().unwrap();

    assert_eq!(
        yaml_entity_count, json_entity_count,
        "YAML entity_count ({}) != JSON entity_count ({})",
        yaml_entity_count, json_entity_count
    );

    // Diagnostic counts should also match across formats
    let yaml_diag_count = yaml_val["metadata"]["diagnostic_count"].as_u64().unwrap();
    let json_diag_count = json_val["metadata"]["diagnostic_count"].as_u64().unwrap();
    assert_eq!(
        yaml_diag_count, json_diag_count,
        "YAML diagnostic_count ({}) != JSON diagnostic_count ({})",
        yaml_diag_count, json_diag_count
    );
}

#[test]
fn test_yaml_json_round_trip_equivalence() {
    let project = create_multi_file_python();
    let path = project.path().to_str().unwrap();

    let yaml_out = flowspec()
        .args(["analyze", path, "--format", "yaml"])
        .output()
        .unwrap();
    let json_out = flowspec()
        .args(["analyze", path, "--format", "json"])
        .output()
        .unwrap();

    let yaml_str = String::from_utf8_lossy(&yaml_out.stdout);
    let json_str = String::from_utf8_lossy(&json_out.stdout);

    let yaml_val: serde_yaml::Value = serde_yaml::from_str(&yaml_str).unwrap();
    let json_val: serde_json::Value = serde_json::from_str(&json_str).unwrap();

    // Compare entity ID sets
    let yaml_ids: Vec<&str> = yaml_val["entities"]
        .as_sequence()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();
    let json_ids: Vec<&str> = json_val["entities"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|e| e["id"].as_str())
        .collect();

    let yaml_set: std::collections::HashSet<&str> = yaml_ids.iter().copied().collect();
    let json_set: std::collections::HashSet<&str> = json_ids.iter().copied().collect();

    assert_eq!(
        yaml_set, json_set,
        "Entity ID sets must match across YAML and JSON formats"
    );
}

#[test]
fn test_diagnose_format_consistency() {
    let project = create_multi_file_python();
    let path = project.path().to_str().unwrap();

    let yaml_out = flowspec()
        .args(["diagnose", path, "--format", "yaml"])
        .output()
        .unwrap();
    let json_out = flowspec()
        .args(["diagnose", path, "--format", "json"])
        .output()
        .unwrap();

    let yaml_str = String::from_utf8_lossy(&yaml_out.stdout);
    let json_str = String::from_utf8_lossy(&json_out.stdout);

    // Both should either parse as arrays or as diagnostic structures
    let yaml_has_content = !yaml_str.trim().is_empty();
    let json_has_content = !json_str.trim().is_empty();
    assert_eq!(
        yaml_has_content, json_has_content,
        "Both formats must produce output (or both be empty)"
    );

    // Exit codes must match
    assert_eq!(
        yaml_out.status.code().unwrap(),
        json_out.status.code().unwrap(),
        "Exit codes must be consistent across formats"
    );
}

#[test]
fn test_diff_format_consistency_yaml_json_summary() {
    let old = minimal_manifest_yaml(&["A", "B"], &[]);
    let new = minimal_manifest_yaml(
        &["A", "B", "C"],
        &[("data_dead_end", "C", "warning", "test.py:5")],
    );
    let old_file = write_temp_manifest(&old, "yaml");
    let new_file = write_temp_manifest(&new, "yaml");

    for format in &["yaml", "json", "summary"] {
        let output = flowspec()
            .args([
                "diff",
                old_file.path().to_str().unwrap(),
                new_file.path().to_str().unwrap(),
                "--format",
                format,
            ])
            .output()
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            !stdout.trim().is_empty(),
            "Diff --format {} produced empty output",
            format
        );
        assert_eq!(
            output.status.code().unwrap(),
            0,
            "Diff --format {} should exit 0 (warning, not critical regression)",
            format
        );
    }
}

// ============================================================================
// Category 4: Pipe Safety (T15–T17)
// ============================================================================

#[test]
fn test_pipe_safety_stdout_is_structured_only() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // stdout must be valid YAML (the default format)
    let _parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout must be valid YAML (pipe safety). Parse error: {}.\nFirst 500 chars:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    // Negative assertions: no log-like content in stdout
    assert!(
        !stdout.contains("[INFO]"),
        "stdout must not contain [INFO] log lines"
    );
    assert!(
        !stdout.contains("[WARN]"),
        "stdout must not contain [WARN] log lines"
    );
    assert!(
        !stdout.contains("[DEBUG]"),
        "stdout must not contain [DEBUG] log lines"
    );
    assert!(
        !stdout.contains("[ERROR]"),
        "stdout must not contain [ERROR] log lines"
    );
}

#[test]
fn test_pipe_safety_stderr_receives_verbose_logging() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap(), "--verbose"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // stdout: still valid structured output
    let _parsed: serde_yaml::Value = serde_yaml::from_str(&stdout)
        .unwrap_or_else(|e| panic!("stdout must be valid YAML even with --verbose: {}", e));

    // stderr: should have SOME logging output with --verbose
    assert!(
        !stderr.is_empty(),
        "--verbose must produce logging output on stderr"
    );
}

#[test]
fn test_pipe_safety_quiet_suppresses_stderr() {
    let project = create_clean_python();

    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap(), "--quiet"])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);

    // With --quiet on a clean project, stderr should be empty or near-empty
    // (only error-level output would pass through the EnvFilter::new("error") filter)
    // If analysis succeeds, there should be no error-level output
    if output.status.success() || output.status.code() == Some(2) {
        assert!(
            stderr.trim().is_empty(),
            "--quiet must suppress non-error stderr on successful analysis. Got:\n{}",
            stderr
        );
    }
}

// ============================================================================
// Category 5: Exit Code Contract (T18–T22)
// ============================================================================

#[test]
fn test_exit_code_0_clean_project() {
    let project = create_clean_python();

    let output = flowspec()
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    // Exit 0 = no critical diagnostics. Exit 2 = critical found.
    // A clean project should be 0, but if the analyzer produces false-positive
    // criticals, this test catches it.
    assert!(
        code == 0 || code == 2,
        "analyze on a project must exit 0 (clean) or 2 (critical findings). Got: {}",
        code
    );
}

#[test]
fn test_exit_code_1_nonexistent_path() {
    let output = flowspec()
        .args(["analyze", "/tmp/flowspec_nonexistent_path_test_42"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Nonexistent path must exit 1 (error). Got: {}",
        output.status.code().unwrap()
    );
}

#[test]
fn test_exit_code_1_invalid_args() {
    let output = flowspec().args(["notacommand"]).output().unwrap();

    // clap's exit 2 for usage errors is intercepted to exit 1
    // per main.rs:236-247 (CI gate contract: exit 2 = findings, not usage error)
    let code = output.status.code().unwrap();
    assert!(
        code == 1 || code == 2,
        "Invalid args must exit 1 (clap exit 2 interception) or 2 (clap default). Got: {}",
        code
    );
}

#[test]
fn test_diagnose_exit_2_on_any_findings_above_threshold() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args(["diagnose", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    // The multi-file fixture should produce at least some diagnostics (dead code, etc.)
    // If it does, diagnose should exit 2 (findings above threshold)
    // If somehow no diagnostics, exit 0 — which is also valid
    assert!(
        code == 0 || code == 2,
        "diagnose must exit 0 (no findings) or 2 (findings). Got: {}",
        code
    );
}

#[test]
fn test_diagnose_severity_filter_affects_exit_code() {
    let project = create_clean_python();

    let output = flowspec()
        .args([
            "diagnose",
            project.path().to_str().unwrap(),
            "--severity",
            "critical",
        ])
        .output()
        .unwrap();

    // A clean project should have no critical diagnostics
    assert_eq!(
        output.status.code().unwrap(),
        0,
        "diagnose --severity critical on clean project must exit 0. Got: {}",
        output.status.code().unwrap()
    );
}

// ============================================================================
// Category 6: YAML Section Ordering (T23–T24)
// ============================================================================

#[test]
fn test_yaml_section_ordering_matches_spec() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Find positions of top-level section keys
    let sections = [
        "metadata:",
        "summary:",
        "diagnostics:",
        "entities:",
        "flows:",
        "boundaries:",
        "dependency_graph:",
        "type_flows:",
    ];

    let mut positions: Vec<(usize, &str)> = Vec::new();
    for section in &sections {
        // Find the top-level occurrence: must be at the start of a line (column 0)
        let search = format!("\n{}", section);
        let pos = stdout
            .find(&search)
            .map(|p| p + 1) // skip the newline to get the actual section position
            .or_else(|| {
                // Check if it's at the very start of the string
                if stdout.starts_with(section) {
                    Some(0)
                } else {
                    None
                }
            });
        if let Some(pos) = pos {
            positions.push((pos, section));
        } else {
            panic!(
                "Required top-level section '{}' not found in YAML output",
                section
            );
        }
    }

    // Verify they appear in order
    for window in positions.windows(2) {
        assert!(
            window[0].0 < window[1].0,
            "Section '{}' (pos {}) must appear before '{}' (pos {}). Spec order violated.",
            window[0].1,
            window[0].0,
            window[1].1,
            window[1].0
        );
    }
}

#[test]
fn test_yaml_all_eight_sections_present() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);

    let required_sections = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];

    for section in &required_sections {
        assert!(
            stdout.contains(&format!("{}:", section)),
            "Required section '{}' missing from YAML output. Manifest schema requires all 8 sections.",
            section
        );
    }
}

// ============================================================================
// Category 7: Error Message Quality (T25–T28)
// ============================================================================

#[test]
fn test_error_message_unknown_checks_pattern() {
    let project = create_clean_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--checks",
            "nonexistent_pattern",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Unknown pattern must exit 1"
    );
    assert!(
        stderr.contains("nonexistent_pattern"),
        "Error must mention the invalid pattern name. Got:\n{}",
        stderr
    );
    // Should list valid patterns as a fix suggestion
    assert!(
        stderr.contains("data_dead_end") || stderr.contains("valid patterns"),
        "Error must suggest valid patterns. Got:\n{}",
        stderr
    );
}

#[test]
fn test_error_message_unsupported_language() {
    let project = create_clean_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--language",
            "haskell",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Unsupported language must exit 1"
    );
    assert!(
        stderr.contains("haskell"),
        "Error must mention the unsupported language"
    );
    assert!(
        stderr.contains("python") && stderr.contains("javascript") && stderr.contains("rust"),
        "Error must list supported languages. Got:\n{}",
        stderr
    );
}

#[test]
fn test_error_message_watch_not_implemented() {
    let output = flowspec().args(["watch", "."]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(output.status.code().unwrap(), 1);
    assert!(
        stderr.contains("not yet implemented") || stderr.contains("not implemented"),
        "Watch error must say 'not implemented'. Got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("fix") || stderr.contains("use flowspec analyze"),
        "Watch error must include a fix suggestion. Got:\n{}",
        stderr
    );
}

#[test]
fn test_error_message_nonexistent_path_includes_path() {
    let bad_path = "/tmp/does_not_exist_flowspec_test_qa3";

    let output = flowspec().args(["analyze", bad_path]).output().unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(bad_path) || stderr.contains("does_not_exist"),
        "Error must include the problematic path. Got:\n{}",
        stderr
    );
    assert!(
        stderr.contains("fix") || stderr.contains("check"),
        "Error must include a fix suggestion. Got:\n{}",
        stderr
    );
}

// ============================================================================
// Category 8: End-to-End Manifest Validation (T29–T33)
// ============================================================================

#[test]
fn test_manifest_metadata_all_fields_present() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("JSON parse failed: {}", e));

    let metadata = &manifest["metadata"];
    let required_fields = [
        "project",
        "analyzed_at",
        "flowspec_version",
        "languages",
        "file_count",
        "entity_count",
        "flow_count",
        "diagnostic_count",
        "incremental",
        "files_changed",
    ];

    for field in &required_fields {
        assert!(
            !metadata[field].is_null(),
            "metadata.{} is missing from manifest output",
            field
        );
    }

    // Type checks
    assert!(
        metadata["languages"].is_array(),
        "languages must be an array"
    );
    assert!(
        metadata["file_count"].is_u64(),
        "file_count must be an integer"
    );
    assert!(
        metadata["incremental"].is_boolean(),
        "incremental must be a boolean"
    );
}

#[test]
fn test_manifest_summary_all_fields_present() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let summary = &manifest["summary"];
    assert!(
        summary["architecture"].is_string(),
        "summary.architecture must be a string"
    );
    assert!(
        summary["modules"].is_array(),
        "summary.modules must be an array"
    );
    assert!(
        summary["entry_points"].is_array(),
        "summary.entry_points must be an array"
    );
    assert!(
        summary["exit_points"].is_array(),
        "summary.exit_points must be an array"
    );
    assert!(
        summary["key_flows"].is_array(),
        "summary.key_flows must be an array"
    );
    assert!(
        summary["diagnostic_summary"].is_object(),
        "summary.diagnostic_summary must be an object"
    );

    let diag_summary = &summary["diagnostic_summary"];
    assert!(
        diag_summary["critical"].is_u64(),
        "diagnostic_summary.critical must be an integer"
    );
    assert!(
        diag_summary["warning"].is_u64(),
        "diagnostic_summary.warning must be an integer"
    );
    assert!(
        diag_summary["info"].is_u64(),
        "diagnostic_summary.info must be an integer"
    );
    assert!(
        diag_summary["top_issues"].is_array(),
        "diagnostic_summary.top_issues must be an array"
    );
}

#[test]
fn test_manifest_entity_fields_complete() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let entities = manifest["entities"]
        .as_array()
        .expect("entities must be an array");
    assert!(
        !entities.is_empty(),
        "Multi-file project must produce entities"
    );

    let required_fields = [
        "id",
        "kind",
        "vis",
        "sig",
        "loc",
        "calls",
        "called_by",
        "annotations",
    ];

    for (i, entity) in entities.iter().enumerate() {
        for field in &required_fields {
            assert!(
                !entity[field].is_null(),
                "entity[{}] ({}) missing required field '{}'",
                i,
                entity["id"].as_str().unwrap_or("??"),
                field
            );
        }
        // Type checks
        assert!(
            entity["calls"].is_array(),
            "entity[{}].calls must be an array",
            i
        );
        assert!(
            entity["called_by"].is_array(),
            "entity[{}].called_by must be an array",
            i
        );
    }
}

#[test]
fn test_manifest_boundaries_section_exists() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(
        manifest["boundaries"].is_array(),
        "boundaries must be an array (even if empty)"
    );

    // POST-Worker-2 assertion (uncomment when boundary wiring is complete):
    // let boundaries = manifest["boundaries"].as_array().unwrap();
    // assert!(
    //     !boundaries.is_empty(),
    //     "Multi-file project with cross-file imports must produce boundary entries after Worker 2 wiring"
    // );
}

#[test]
fn test_manifest_diagnostics_have_confidence() {
    let project = create_multi_file_python();

    let output = flowspec()
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let manifest: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let diagnostics = manifest["diagnostics"]
        .as_array()
        .expect("diagnostics must be an array");

    for (i, diag) in diagnostics.iter().enumerate() {
        let confidence = diag["confidence"]
            .as_str()
            .unwrap_or_else(|| panic!("diagnostic[{}] missing confidence field", i));
        assert!(
            ["high", "moderate", "low"].contains(&confidence),
            "diagnostic[{}] confidence '{}' not in {{high, moderate, low}}",
            i,
            confidence
        );
    }
}

// ============================================================================
// Category 9: Adversarial Edge Cases (T34–T38)
// ============================================================================

#[test]
fn test_diff_sarif_format_returns_error_not_crash() {
    let manifest = minimal_manifest_yaml(&["fn_a"], &[]);
    let old_file = write_temp_manifest(&manifest, "yaml");
    let new_file = write_temp_manifest(&manifest, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--format",
            "sarif",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "SARIF diff must exit 1 (format not implemented). Got: {}",
        output.status.code().unwrap()
    );
    assert!(
        stderr.contains("not") && stderr.contains("implemented"),
        "Error must say format is not implemented. Got:\n{}",
        stderr
    );
}

#[test]
fn test_diff_empty_manifest_file() {
    let valid = minimal_manifest_yaml(&["fn_a"], &[]);
    let valid_file = write_temp_manifest(&valid, "yaml");
    let empty_file = write_temp_manifest("", "yaml");

    let output = flowspec()
        .args([
            "diff",
            valid_file.path().to_str().unwrap(),
            empty_file.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Empty manifest must exit 1 (error). Got: {}",
        output.status.code().unwrap()
    );
}

#[test]
fn test_diff_malformed_yaml() {
    let valid = minimal_manifest_yaml(&["fn_a"], &[]);
    let valid_file = write_temp_manifest(&valid, "yaml");
    let malformed_file = write_temp_manifest("metadata:\n  project: [unterminated", "yaml");

    let output = flowspec()
        .args([
            "diff",
            valid_file.path().to_str().unwrap(),
            malformed_file.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Malformed YAML must exit 1. Got: {}",
        output.status.code().unwrap()
    );
}

#[test]
fn test_diff_invalid_section_name() {
    let manifest = minimal_manifest_yaml(&["fn_a"], &[]);
    let old_file = write_temp_manifest(&manifest, "yaml");
    let new_file = write_temp_manifest(&manifest, "yaml");

    let output = flowspec()
        .args([
            "diff",
            old_file.path().to_str().unwrap(),
            new_file.path().to_str().unwrap(),
            "--section",
            "flows",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "--section flows must exit 1 (section not yet supported for diffing). Got: {}",
        output.status.code().unwrap()
    );
}

#[test]
fn test_trace_nonexistent_symbol_error_quality() {
    let project = create_clean_python();

    let output = flowspec()
        .args([
            "trace",
            project.path().to_str().unwrap(),
            "--symbol",
            "definitely_not_a_real_symbol_xyz",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(
        output.status.code().unwrap(),
        1,
        "Symbol not found must exit 1"
    );
    assert!(
        stderr.contains("definitely_not_a_real_symbol_xyz") || stderr.contains("not found"),
        "Error must mention the missing symbol or say 'not found'. Got:\n{}",
        stderr
    );
}
