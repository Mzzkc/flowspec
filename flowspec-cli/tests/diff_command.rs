//! QA-3 tests for the `flowspec diff` command — Cycle 18.
//!
//! 28 tests across 8 categories per tests-3.md spec.
//! TDD anchors: T1-T18, T20-T21, T23-T25.

use assert_cmd::Command;
use predicates::prelude::*;
use std::io::Write;
use tempfile::NamedTempFile;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

/// Minimal valid YAML manifest for testing.
fn minimal_manifest_yaml(entities: &[&str], diagnostics: &[(&str, &str, &str, &str)]) -> String {
    let mut yaml = String::new();
    yaml.push_str("metadata:\n");
    yaml.push_str("  project: test-project\n");
    yaml.push_str("  analyzed_at: '2026-03-26T00:00:00Z'\n");
    yaml.push_str("  flowspec_version: '0.1.0'\n");
    yaml.push_str("  languages:\n    - python\n");
    yaml.push_str(&format!("  file_count: 1\n"));
    yaml.push_str(&format!("  entity_count: {}\n", entities.len()));
    yaml.push_str("  flow_count: 0\n");
    yaml.push_str(&format!("  diagnostic_count: {}\n", diagnostics.len()));
    yaml.push_str("  incremental: false\n");
    yaml.push_str("  files_changed: 0\n");
    yaml.push_str("summary:\n");
    yaml.push_str("  architecture: Test project\n");
    yaml.push_str("  modules: []\n");
    yaml.push_str("  entry_points: []\n");
    yaml.push_str("  exit_points: []\n");
    yaml.push_str("  key_flows: []\n");
    yaml.push_str("  diagnostic_summary:\n");
    yaml.push_str("    critical: 0\n");
    yaml.push_str("    warning: 0\n");
    yaml.push_str("    info: 0\n");
    yaml.push_str("    top_issues: []\n");
    yaml.push_str("diagnostics:\n");
    if diagnostics.is_empty() {
        yaml.push_str("  []\n");
    } else {
        for (i, (pattern, entity, severity, loc)) in diagnostics.iter().enumerate() {
            yaml.push_str(&format!("  - id: D{:03}\n", i + 1));
            yaml.push_str(&format!("    pattern: {}\n", pattern));
            yaml.push_str(&format!("    severity: {}\n", severity));
            yaml.push_str("    confidence: high\n");
            yaml.push_str(&format!("    entity: {}\n", entity));
            yaml.push_str("    message: Test diagnostic\n");
            yaml.push_str("    evidence:\n");
            yaml.push_str("      - observation: test observation\n");
            yaml.push_str("    suggestion: Fix it\n");
            yaml.push_str(&format!("    loc: {}\n", loc));
        }
    }
    yaml.push_str("entities:\n");
    if entities.is_empty() {
        yaml.push_str("  []\n");
    } else {
        for id in entities {
            yaml.push_str(&format!("  - id: {}\n", id));
            yaml.push_str("    kind: fn\n");
            yaml.push_str("    vis: pub\n");
            yaml.push_str("    sig: '() -> None'\n");
            yaml.push_str(&format!("    loc: test.py:1\n"));
            yaml.push_str("    calls: []\n");
            yaml.push_str("    called_by: []\n");
            yaml.push_str("    annotations: []\n");
        }
    }
    yaml.push_str("flows: []\n");
    yaml.push_str("boundaries: []\n");
    yaml.push_str("dependency_graph: []\n");
    yaml.push_str("type_flows: []\n");
    yaml
}

/// Write manifest content to a temp file with the given extension.
fn write_temp_manifest(content: &str, extension: &str) -> NamedTempFile {
    let mut file = tempfile::Builder::new()
        .suffix(&format!(".{}", extension))
        .tempfile()
        .unwrap();
    file.write_all(content.as_bytes()).unwrap();
    file.flush().unwrap();
    file
}

/// Convert YAML manifest string to JSON manifest string.
fn yaml_to_json(yaml_content: &str) -> String {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml_content).unwrap();
    serde_json::to_string_pretty(&value).unwrap()
}

// ============================================================
// Category 1: Diff CLI Parsing (T1-T5)
// ============================================================

// T1: diff command accepted with two paths
#[test]
fn diff_command_accepted_with_two_paths() {
    let old = write_temp_manifest(&minimal_manifest_yaml(&[], &[]), "yaml");
    let new = write_temp_manifest(&minimal_manifest_yaml(&[], &[]), "yaml");

    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert();
    // Should NOT be a clap usage error
    assert.stderr(predicate::str::contains("Usage").not());
}

// T2: diff requires two path arguments
#[test]
fn diff_requires_two_path_arguments() {
    flowspec().args(["diff", "/tmp/a.yaml"]).assert().failure();
}

// T3: diff rejects zero arguments
#[test]
fn diff_rejects_zero_arguments() {
    flowspec().args(["diff"]).assert().failure();
}

// T4: diff help shows old/new args and section flag
#[test]
fn diff_help_shows_old_new_args_and_section_flag() {
    flowspec()
        .args(["diff", "--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("OLD"))
        .stdout(predicate::str::contains("NEW"))
        .stdout(predicate::str::contains("--section"));
}

// T5: diff section flag accepted
#[test]
fn diff_section_flag_accepted() {
    let old = write_temp_manifest(&minimal_manifest_yaml(&[], &[]), "yaml");
    let new = write_temp_manifest(&minimal_manifest_yaml(&[], &[]), "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
            "--section",
            "entities",
        ])
        .assert()
        .stderr(predicate::str::contains("Usage").not());
}

// ============================================================
// Category 2: Diff Happy Path (T6-T10)
// ============================================================

// T6: diff identical manifests exit 0
#[test]
fn diff_identical_manifests_exit_0() {
    let content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let old = write_temp_manifest(&content, "yaml");
    let new = write_temp_manifest(&content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no changes"));
}

// T7: diff added entity shows in output
#[test]
fn diff_added_entity_shows_in_output() {
    let old_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let new_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b", "mod::func_c"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("func_c"));
}

// T8: diff removed entity shows in output
#[test]
fn diff_removed_entity_shows_in_output() {
    let old_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b", "mod::func_c"], &[]);
    let new_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("func_c"));
}

// T9: diff new critical diagnostic exit 2
#[test]
fn diff_new_critical_diagnostic_exit_2() {
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(
        &["mod::func_a"],
        &[("data_dead_end", "mod::func_a", "critical", "test.py:1")],
    );
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .code(2);
}

// T10: diff resolved diagnostic shown
#[test]
fn diff_resolved_diagnostic_shown() {
    let old_content = minimal_manifest_yaml(
        &["mod::func_a"],
        &[("data_dead_end", "mod::func_a", "warning", "test.py:1")],
    );
    let new_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("resolved")
                .or(predicate::str::contains("diagnostics_resolved")),
        );
}

// ============================================================
// Category 3: Diff Error Handling (T11-T15)
// ============================================================

// T11: diff nonexistent old manifest exit 1
#[test]
fn diff_nonexistent_old_manifest_exit_1() {
    let new_content = minimal_manifest_yaml(&[], &[]);
    let new = write_temp_manifest(&new_content, "yaml");

    flowspec()
        .args([
            "diff",
            "/nonexistent/old.yaml",
            new.path().to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("/nonexistent/old.yaml"));
}

// T12: diff nonexistent new manifest exit 1
#[test]
fn diff_nonexistent_new_manifest_exit_1() {
    let old_content = minimal_manifest_yaml(&[], &[]);
    let old = write_temp_manifest(&old_content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            "/nonexistent/new.yaml",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("/nonexistent/new.yaml"));
}

// T13: diff invalid YAML manifest exit 1
#[test]
fn diff_invalid_yaml_manifest_exit_1() {
    let invalid = write_temp_manifest("{{{not yaml", "yaml");
    let valid_content = minimal_manifest_yaml(&[], &[]);
    let valid = write_temp_manifest(&valid_content, "yaml");

    flowspec()
        .args([
            "diff",
            invalid.path().to_str().unwrap(),
            valid.path().to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("could not parse manifest"));
}

// T14: diff truncated JSON manifest exit 1
#[test]
fn diff_truncated_json_manifest_exit_1() {
    let truncated = write_temp_manifest("{\"metadata\": {", "json");
    let valid_content = minimal_manifest_yaml(&[], &[]);
    let valid = write_temp_manifest(&valid_content, "yaml");

    flowspec()
        .args([
            "diff",
            truncated.path().to_str().unwrap(),
            valid.path().to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("could not parse manifest"));
}

// T15: diff empty file exit 1
#[test]
fn diff_empty_file_exit_1() {
    let empty = write_temp_manifest("", "yaml");
    let valid_content = minimal_manifest_yaml(&[], &[]);
    let valid = write_temp_manifest(&valid_content, "yaml");

    flowspec()
        .args([
            "diff",
            empty.path().to_str().unwrap(),
            valid.path().to_str().unwrap(),
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("empty").or(predicate::str::contains("could not parse")));
}

// ============================================================
// Category 4: Section Filtering (T16-T19)
// ============================================================

// T16: diff section entities only
#[test]
fn diff_section_entities_only() {
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(
        &["mod::func_a", "mod::func_b"],
        &[("data_dead_end", "mod::func_b", "warning", "test.py:5")],
    );
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
            "--section",
            "entities",
        ])
        .assert();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Should show entity changes
    assert!(
        stdout.contains("func_b"),
        "Should show added entity func_b, got: {}",
        stdout
    );
    // Should NOT show diagnostic changes (filtered out)
    assert!(
        !stdout.contains("diagnostics_new")
            || stdout.contains("diagnostics_new: []")
            || stdout.contains("diagnostics_new:\n- []"),
        "Should not show non-empty diagnostic changes when filtering to entities only, got: {}",
        stdout
    );
}

// T17: diff multiple section flags
#[test]
fn diff_multiple_section_flags() {
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(
        &["mod::func_a", "mod::func_b"],
        &[("data_dead_end", "mod::func_b", "warning", "test.py:5")],
    );
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
            "--section",
            "entities",
            "--section",
            "diagnostics",
        ])
        .assert();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Should show both entity and diagnostic changes
    assert!(
        stdout.contains("func_b"),
        "Should show entity changes, got: {}",
        stdout
    );
    assert!(
        stdout.contains("data_dead_end"),
        "Should show diagnostic changes, got: {}",
        stdout
    );
}

// T18: diff invalid section name exit 1
#[test]
fn diff_invalid_section_name_exit_1() {
    let content = minimal_manifest_yaml(&[], &[]);
    let old = write_temp_manifest(&content, "yaml");
    let new = write_temp_manifest(&content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
            "--section",
            "nonexistent_section",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("nonexistent_section"))
        .stderr(predicate::str::contains("valid sections"));
}

// T19: diff section flag with no value rejected
#[test]
fn diff_section_flag_with_no_value_rejected() {
    flowspec()
        .args(["diff", "/tmp/a.yaml", "/tmp/b.yaml", "--section"])
        .assert()
        .failure();
}

// ============================================================
// Category 5: Edge Cases (T20-T23)
// ============================================================

// T20: diff same file against itself exit 0
#[test]
fn diff_same_file_against_itself_exit_0() {
    let content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let file = write_temp_manifest(&content, "yaml");

    flowspec()
        .args([
            "diff",
            file.path().to_str().unwrap(),
            file.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no changes"));
}

// T21: diff YAML vs JSON cross-format
#[test]
fn diff_yaml_vs_json_cross_format() {
    let yaml_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let json_content = yaml_to_json(&yaml_content);
    let yaml_file = write_temp_manifest(&yaml_content, "yaml");
    let json_file = write_temp_manifest(&json_content, "json");

    flowspec()
        .args([
            "diff",
            yaml_file.path().to_str().unwrap(),
            json_file.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no changes"));
}

// T22: diff empty manifests exit 0
#[test]
fn diff_empty_manifests_exit_0() {
    let content = minimal_manifest_yaml(&[], &[]);
    let old = write_temp_manifest(&content, "yaml");
    let new = write_temp_manifest(&content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success()
        .stdout(predicate::str::contains("no changes"));
}

// T23: diff manifest missing optional fields
#[test]
fn diff_manifest_missing_optional_fields() {
    // Manifests with different entity counts but valid structure
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    // Should handle gracefully (exit 0 or 2, NOT exit 1)
    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert();
    let exit_code = assert.get_output().status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "Expected exit 0 or 2, got {}",
        exit_code
    );
}

// ============================================================
// Category 6: Pipe Safety + Output Format (T24-T25)
// ============================================================

// T24: diff stdout is pipe safe
#[test]
fn diff_stdout_is_pipe_safe() {
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // stdout must NOT contain tracing output
    assert!(
        !stdout.contains("INFO"),
        "stdout must not contain tracing INFO lines"
    );
    assert!(
        !stdout.contains("DEBUG"),
        "stdout must not contain tracing DEBUG lines"
    );
    assert!(
        !stdout.contains("WARN"),
        "stdout must not contain tracing WARN lines"
    );
}

// T25: diff format flag respected (JSON)
#[test]
fn diff_format_flag_respected() {
    let old_content = minimal_manifest_yaml(&["mod::func_a"], &[]);
    let new_content = minimal_manifest_yaml(&["mod::func_a", "mod::func_b"], &[]);
    let old = write_temp_manifest(&old_content, "yaml");
    let new = write_temp_manifest(&new_content, "yaml");

    let assert = flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
            "--format",
            "json",
        ])
        .assert()
        .success();

    let stdout = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // Must be valid JSON
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout must be valid JSON when --format json is used, got: {}",
        stdout
    );
}

// ============================================================
// Category 7: Regression Guards (T26-T28)
// ============================================================

// T26: existing commands unaffected by diff addition
#[test]
fn existing_commands_unaffected_by_diff_addition() {
    // analyze on current dir should work (exit 0 or 2)
    let assert = flowspec()
        .args(["analyze", "."])
        .current_dir(workspace_root())
        .assert();
    let exit_code = assert.get_output().status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "analyze should exit 0 or 2, got {}",
        exit_code
    );

    // init should work
    let tmp = tempfile::tempdir().unwrap();
    flowspec()
        .args(["init", tmp.path().to_str().unwrap()])
        .assert()
        .success();
}

// T27: help lists diff command
#[test]
fn help_lists_diff_command() {
    flowspec()
        .args(["--help"])
        .assert()
        .success()
        .stdout(predicate::str::contains("diff"));
}

// T28: deferred commands loop updated (diff no longer returns not implemented)
#[test]
fn diff_no_longer_returns_not_implemented() {
    let content = minimal_manifest_yaml(&[], &[]);
    let old = write_temp_manifest(&content, "yaml");
    let new = write_temp_manifest(&content, "yaml");

    flowspec()
        .args([
            "diff",
            old.path().to_str().unwrap(),
            new.path().to_str().unwrap(),
        ])
        .assert()
        .stderr(predicate::str::contains("not yet implemented").not());
}

/// Get workspace root for integration tests.
fn workspace_root() -> std::path::PathBuf {
    let output = std::process::Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version=1"])
        .output()
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    std::path::PathBuf::from(metadata["workspace_root"].as_str().unwrap())
}
