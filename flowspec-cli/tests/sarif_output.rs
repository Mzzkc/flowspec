//! Tests for SARIF v2.1.0 output formatter — structure, severity, CLI integration.
//!
//! SARIF output enables GitHub Code Scanning integration. These tests verify
//! schema compliance, severity mapping, and composition with other CLI flags.

use std::process::Command;

/// Get the flowspec binary path.
fn flowspec_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flowspec"))
}

/// Create a temp directory with a Python file that produces diagnostics.
/// Includes both dead functions AND called functions so data_dead_end fires.
fn create_project_with_diagnostics() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("dead_code.py"),
        "def unused_helper(x):\n    return x * 2\n\n\
         def _private_util():\n    return 42\n\n\
         def active_function(data):\n    return data.strip()\n\n\
         def main_handler(request):\n    result = active_function(request)\n    return result\n",
    )
    .unwrap();
    dir
}

/// Create a temp directory with clean Python code (no diagnostics).
fn create_clean_project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("clean.py"),
        "def main():\n    greet()\n\ndef greet():\n    print('hello')\n\nmain()\n",
    )
    .unwrap();
    dir
}

// =============================================================================
// 3. SARIF Structure Tests
// =============================================================================

#[test]
fn sarif_output_valid_json() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "SARIF output must be valid JSON, got error: {:?}\nstdout:\n{}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
    let exit_code = output.status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "Exit code should be 0 or 2, got: {}",
        exit_code
    );
}

#[test]
fn sarif_has_required_top_level_fields() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert!(
        parsed["$schema"].as_str().unwrap().contains("sarif"),
        "$schema must reference SARIF spec"
    );
    assert_eq!(parsed["version"], "2.1.0", "version must be 2.1.0");
    assert!(parsed["runs"].is_array(), "runs must be an array");
    assert!(
        !parsed["runs"].as_array().unwrap().is_empty(),
        "runs must not be empty"
    );
}

#[test]
fn sarif_run_contains_tool_driver() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let driver = &parsed["runs"][0]["tool"]["driver"];

    assert_eq!(driver["name"], "Flowspec", "driver.name must be 'Flowspec'");
    assert!(
        driver["version"].is_string(),
        "driver.version must be a string"
    );
    assert!(driver["rules"].is_array(), "driver.rules must be an array");
}

#[test]
fn sarif_diagnostics_map_to_results() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    assert!(!results.is_empty(), "dead code should produce diagnostics");
    for result in results {
        assert!(result["ruleId"].is_string(), "Each result must have ruleId");
        let level = result["level"].as_str().unwrap();
        assert!(
            ["error", "warning", "note"].contains(&level),
            "level must be error/warning/note, got: {}",
            level
        );
        assert!(
            result["message"]["text"].is_string(),
            "message.text must be a string"
        );
        assert!(
            result["locations"].is_array() && !result["locations"].as_array().unwrap().is_empty(),
            "locations must be a non-empty array"
        );
    }
}

// =============================================================================
// 3.5-3.7 SARIF Severity Mapping
// =============================================================================

#[test]
fn sarif_severity_mapping_via_diagnostics() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    for result in results {
        let level = result["level"].as_str().unwrap();
        assert!(
            ["error", "warning", "note"].contains(&level),
            "Invalid SARIF level: {}",
            level
        );
    }
}

// =============================================================================
// 3.8 SARIF Location Structure
// =============================================================================

#[test]
fn sarif_location_has_artifact_and_region() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    for result in results {
        let loc = &result["locations"][0]["physicalLocation"];
        assert!(
            loc["artifactLocation"]["uri"].is_string(),
            "artifactLocation.uri must be present"
        );
        if loc.get("region").is_some() {
            assert!(
                loc["region"]["startLine"].is_number(),
                "region.startLine must be a number"
            );
        }
    }
}

// =============================================================================
// 3.9 SARIF Rules
// =============================================================================

#[test]
fn sarif_rules_match_result_rule_ids() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let rules = parsed["runs"][0]["tool"]["driver"]["rules"]
        .as_array()
        .unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    let rule_ids: std::collections::HashSet<&str> =
        rules.iter().map(|r| r["id"].as_str().unwrap()).collect();

    for result in results {
        let rule_id = result["ruleId"].as_str().unwrap();
        assert!(
            rule_ids.contains(rule_id),
            "Result ruleId '{}' has no matching rule definition. Rules: {:?}",
            rule_id,
            rule_ids
        );
    }
}

// =============================================================================
// 3.10 SARIF Diagnose Command
// =============================================================================

#[test]
fn sarif_diagnose_format() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "diagnose",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "diagnose --format sarif must produce valid JSON: {}\nstdout:\n{}",
            e, stdout
        )
    });

    assert_eq!(parsed["version"], "2.1.0");
    assert!(parsed["runs"][0]["results"].is_array());
}

// =============================================================================
// 4. SARIF Adversarial Tests
// =============================================================================

#[test]
fn sarif_zero_diagnostics_valid() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    assert_eq!(parsed["version"], "2.1.0");
    assert!(parsed["$schema"].as_str().unwrap().contains("sarif"));
    let results = parsed["runs"][0]["results"].as_array().unwrap();
    assert!(
        results.is_empty(),
        "Clean code should produce empty results array, got {} results",
        results.len()
    );
}

#[test]
fn sarif_unicode_entity_names() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("unicode.py"),
        "def calcul_répartition():\n    return 42\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args(["--format", "sarif", "analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(parsed.is_ok(), "SARIF with unicode must produce valid JSON");
}

#[test]
fn sarif_stdout_pipe_safe() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout must contain ONLY valid JSON (pipe-safe), got:\n{}",
        &stdout[..stdout.len().min(300)]
    );
}

#[test]
fn sarif_many_diagnostics_valid() {
    let dir = tempfile::tempdir().unwrap();
    // Create many dead-end functions to generate lots of diagnostics
    let mut code = String::new();
    for i in 0..20 {
        code.push_str(&format!("def dead_fn_{}():\n    return {}\n\n", i, i));
    }
    std::fs::write(dir.path().join("many_dead.py"), &code).unwrap();

    let output = flowspec_bin()
        .args(["--format", "sarif", "analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .expect("SARIF with many diagnostics must still be valid JSON");

    let results = parsed["runs"][0]["results"].as_array().unwrap();
    assert!(
        results.len() >= 1,
        "Many dead functions should produce at least 1 diagnostic"
    );
}

// =============================================================================
// 5. SARIF CLI Integration Tests
// =============================================================================

#[test]
fn format_sarif_accepted_analyze() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let exit_code = output.status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "--format sarif must be accepted by analyze (exit 0 or 2), got: {}\nstderr: {}",
        exit_code,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn format_sarif_accepted_diagnose() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "diagnose",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let exit_code = output.status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "--format sarif must be accepted by diagnose (exit 0 or 2), got: {}\nstderr: {}",
        exit_code,
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn sarif_exit_code_zero_clean_project() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "Clean project with SARIF format should exit 0"
    );
}

#[test]
fn sarif_exit_code_two_diagnose_findings() {
    // diagnose uses has_findings (any diagnostic), not has_critical
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "diagnose",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "Diagnose with findings should exit 2 regardless of format"
    );
}

#[test]
fn sarif_exit_code_one_error() {
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            "/nonexistent/path/that/does/not/exist",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "Nonexistent path should exit 1 regardless of format"
    );
}

#[test]
fn sarif_with_language_filter() {
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
            "sarif",
            "analyze",
            "--language",
            "python",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    for result in results {
        let uri = result["locations"][0]["physicalLocation"]["artifactLocation"]["uri"]
            .as_str()
            .unwrap_or("");
        assert!(
            !uri.contains(".js"),
            "SARIF + language filter should not include JS results, got uri: {}",
            uri
        );
    }
}

#[test]
fn sarif_diagnose_with_severity() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "diagnose",
            "--severity",
            "warning",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let results = parsed["runs"][0]["results"].as_array().unwrap();

    for result in results {
        let level = result["level"].as_str().unwrap();
        assert!(
            level == "error" || level == "warning",
            "With --severity warning, should not see 'note' level, got: {}",
            level
        );
    }
}

#[test]
fn sarif_format_dispatch_no_unreachable_panic() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "sarif",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("unreachable"),
        "format_with dispatch must not hit unreachable!() for SARIF"
    );
    assert!(
        !stderr.contains("panicked"),
        "format_with dispatch must not panic for SARIF"
    );
}

// =============================================================================
// 6. Regression Tests
// =============================================================================

#[test]
fn yaml_output_still_works_after_sarif() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "yaml",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("YAML format must still work after SARIF addition");

    let map = parsed.as_mapping().unwrap();
    assert_eq!(map.len(), 8, "YAML manifest must have 8 sections");
}

#[test]
fn json_output_still_works_after_sarif() {
    let project = create_clean_project();
    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "analyze",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("JSON format must still work after SARIF addition");

    let obj = parsed.as_object().unwrap();
    assert_eq!(obj.len(), 8, "JSON manifest must have 8 sections");
}

#[test]
fn analyze_language_still_works() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("example.py"),
        "def hello():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.js"),
        "function hello() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "analyze",
            "--language",
            "python",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    for entity in entities {
        let loc = entity["loc"].as_str().unwrap_or("");
        assert!(
            !loc.contains(".js"),
            "analyze --language python should still filter correctly after changes"
        );
    }
}

#[test]
fn diagnose_checks_filter_still_works() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
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

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = parsed.as_array().unwrap();

    for entry in arr {
        assert_eq!(
            entry["pattern"].as_str().unwrap(),
            "data_dead_end",
            "checks filter must still work after diagnose --language changes"
        );
    }
}

#[test]
fn diagnose_severity_filter_still_works() {
    let project = create_project_with_diagnostics();
    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "diagnose",
            "--severity",
            "critical",
            project.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let arr = parsed.as_array().unwrap();

    for entry in arr {
        assert_eq!(
            entry["severity"].as_str().unwrap(),
            "critical",
            "severity filter must still work after diagnose changes"
        );
    }
}
