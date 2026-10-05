use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

#[test]
fn normal_cli_invalid_language_lists_all_defaults() {
    let dir = TempDir::new().unwrap();
    let out = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--language",
            "invalid",
        ])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let error = String::from_utf8(out.stderr).unwrap();
    for language in ["python", "javascript", "typescript", "rust"] {
        assert!(error.contains(language), "missing {language}: {error}");
    }
}

#[test]
fn normal_cli_reports_full_registry_even_when_pattern_filtered() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let out = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--checks",
            "data_dead_end",
        ])
        .output()
        .unwrap();
    assert!(matches!(out.status.code(), Some(0 | 2 | 3)));
    let report: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        report["provenance"]["registry"]["patterns"]
            .as_array()
            .map(Vec::len),
        Some(13)
    );
    assert_eq!(
        report["provenance"]["registry"]["languages"]
            .as_array()
            .map(Vec::len),
        Some(4)
    );
}
