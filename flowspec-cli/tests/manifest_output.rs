use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn create_clean_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

def main():
    result = greet("world")
    print(result)

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();
    dir
}

fn create_project_with_issues() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        r#"
import os

def active_function():
    return 42

def dead_function():
    """This function is never called by anything."""
    return "unreachable"

def another_dead_function(x):
    return x * 2

def main():
    result = active_function()
    print(result)
"#,
    )
    .unwrap();
    dir
}

#[test]
fn manifest_output_is_valid_yaml() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout is not valid YAML: {:?}\nOutput was:\n{}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn manifest_has_all_eight_required_sections() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let map = parsed
        .as_mapping()
        .expect("manifest root must be a YAML mapping");

    let required_sections = [
        "metadata",
        "summary",
        "entities",
        "flows",
        "boundaries",
        "diagnostics",
        "dependency_graph",
        "type_flows",
    ];

    for section in &required_sections {
        assert!(
            map.contains_key(&serde_yaml::Value::String(section.to_string())),
            "Missing required manifest section: '{}'.",
            section
        );
    }
}

#[test]
fn manifest_sections_present_even_when_empty() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let map = parsed.as_mapping().unwrap();

    for section in &["flows", "boundaries", "type_flows"] {
        let key = serde_yaml::Value::String(section.to_string());
        let val = map
            .get(&key)
            .unwrap_or_else(|| panic!("Section '{}' missing", section));
        assert!(
            val.is_sequence(),
            "Section '{}' must be a list (got {:?}).",
            section,
            val
        );
    }
}

#[test]
fn metadata_has_required_fields() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let metadata = parsed.get("metadata").expect("metadata section missing");

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
            metadata.get(field).is_some(),
            "metadata missing required field: '{}'",
            field
        );
    }
}

#[test]
fn metadata_analyzed_at_is_iso8601() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let analyzed_at = parsed["metadata"]["analyzed_at"]
        .as_str()
        .expect("analyzed_at must be a string");

    assert!(
        analyzed_at.contains('T') || analyzed_at.contains(' '),
        "analyzed_at '{}' doesn't look like ISO 8601",
        analyzed_at
    );
}

#[test]
fn entity_uses_abbreviated_field_names() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let entities = parsed["entities"]
        .as_sequence()
        .expect("entities must be a list");

    assert!(
        !entities.is_empty(),
        "entities list should not be empty for a non-empty project"
    );

    let first = &entities[0];
    for field in &["id", "kind", "vis", "loc"] {
        assert!(
            first.get(field).is_some(),
            "Entity missing abbreviated field '{}'.",
            field
        );
    }

    for bad_field in &["visibility", "signature", "location"] {
        assert!(
            first.get(bad_field).is_none(),
            "Entity uses non-abbreviated field name '{}'.",
            bad_field
        );
    }
}

#[test]
fn entity_kind_uses_abbreviated_values() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_sequence().unwrap();

    let valid_kinds = [
        "fn",
        "method",
        "struct",
        "class",
        "trait",
        "interface",
        "module",
        "var",
        "const",
        "macro",
        "enum",
    ];

    for entity in entities {
        if let Some(kind) = entity.get("kind").and_then(|k| k.as_str()) {
            assert!(
                valid_kinds.contains(&kind),
                "Entity kind '{}' is not in the valid abbreviated set: {:?}",
                kind,
                valid_kinds
            );
        }
    }
}

#[test]
fn entity_loc_format_is_file_colon_line() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_sequence().unwrap();

    for entity in entities {
        if let Some(loc) = entity.get("loc").and_then(|l| l.as_str()) {
            assert!(
                loc.contains(':'),
                "Entity loc '{}' doesn't match file:line format",
                loc
            );
            let parts: Vec<&str> = loc.rsplitn(2, ':').collect();
            assert!(
                parts[0].parse::<u32>().is_ok(),
                "Entity loc '{}' — part after colon '{}' is not a valid line number",
                loc,
                parts[0]
            );
        }
    }
}

#[test]
fn diagnostic_entries_include_confidence() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let diagnostics = parsed["diagnostics"]
        .as_sequence()
        .expect("diagnostics must be a list");

    for diag in diagnostics {
        assert!(
            diag.get("confidence").is_some(),
            "Diagnostic entry missing 'confidence' field.\nDiagnostic: {:?}",
            diag
        );

        let conf = diag["confidence"]
            .as_str()
            .expect("confidence must be a string");
        assert!(
            ["high", "moderate", "low"].contains(&conf),
            "Invalid confidence value '{}'.",
            conf
        );
    }
}

#[test]
fn diagnostic_entries_have_all_required_fields() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let diagnostics = parsed["diagnostics"].as_sequence().unwrap();

    let required_fields = [
        "id",
        "pattern",
        "severity",
        "confidence",
        "entity",
        "message",
        "evidence",
        "suggestion",
        "loc",
    ];

    for diag in diagnostics {
        for field in &required_fields {
            assert!(
                diag.get(field).is_some(),
                "Diagnostic missing required field '{}'. Full diagnostic: {:?}",
                field,
                diag
            );
        }
    }
}

#[test]
fn diagnostic_evidence_is_specific_not_vague() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let diagnostics = parsed["diagnostics"].as_sequence().unwrap();

    for diag in diagnostics {
        if let Some(evidence) = diag.get("evidence").and_then(|e| e.as_str()) {
            assert!(
                evidence.len() > 20,
                "Evidence is suspiciously short ('{}')",
                evidence
            );
            let message = diag.get("message").and_then(|m| m.as_str()).unwrap_or("");
            assert_ne!(
                evidence, message,
                "Evidence is identical to message — evidence must add specificity."
            );
        }
    }
}

#[test]
fn diagnostic_suggestion_is_actionable() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let diagnostics = parsed["diagnostics"].as_sequence().unwrap();

    for diag in diagnostics {
        if let Some(suggestion) = diag.get("suggestion").and_then(|s| s.as_str()) {
            assert!(!suggestion.is_empty(), "Suggestion is empty.");
            assert!(
                suggestion.len() > 10,
                "Suggestion '{}' is too short to be actionable.",
                suggestion
            );
        }
    }
}

#[test]
fn summary_diagnostic_summary_has_severity_counts() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let summary = parsed.get("summary").expect("summary section missing");
    let diag_summary = summary
        .get("diagnostic_summary")
        .expect("summary.diagnostic_summary missing");

    for field in &["critical", "warning", "info"] {
        assert!(
            diag_summary.get(field).is_some(),
            "diagnostic_summary missing '{}' count",
            field
        );
        assert!(
            diag_summary[field].is_number(),
            "diagnostic_summary.{} must be a number",
            field
        );
    }

    assert!(
        diag_summary.get("top_issues").is_some(),
        "diagnostic_summary missing 'top_issues' list"
    );
    assert!(
        diag_summary["top_issues"].is_sequence(),
        "diagnostic_summary.top_issues must be a list"
    );
}

#[test]
fn manifest_does_not_exceed_10x_source_size() {
    let project = create_project_with_issues();
    let source_size: u64 = walkdir::WalkDir::new(project.path())
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().extension().map(|x| x == "py").unwrap_or(false))
        .map(|e| e.metadata().map(|m| m.len()).unwrap_or(0))
        .sum();

    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let manifest_size = output.stdout.len() as u64;
    assert!(
        manifest_size <= source_size * 10,
        "Manifest size ({} bytes) exceeds 10x source size ({} bytes). Ratio: {:.1}x",
        manifest_size,
        source_size,
        manifest_size as f64 / source_size as f64
    );
}

#[test]
fn summary_section_fits_2k_token_budget() {
    let project = create_project_with_issues();
    let mut cmd = Command::cargo_bin("flowspec-cli").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_yaml::Value = serde_yaml::from_str(&stdout).unwrap();
    let summary = parsed.get("summary").expect("summary section missing");

    let summary_yaml = serde_yaml::to_string(&summary).unwrap();
    let summary_chars = summary_yaml.len();
    assert!(
        summary_chars <= 10_000,
        "Summary section is {} chars (~{} tokens). Must fit ~2K token budget.",
        summary_chars,
        summary_chars / 4
    );
}
