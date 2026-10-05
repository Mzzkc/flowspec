use flowspec::{analyze, Config, OutputFormatter, SarifFormatter};
use std::fs;
use tempfile::TempDir;

#[test]
fn sarif_results_retain_authoritative_findings_after_display_filtering() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.py"),
        "def _unconsumed():\n    return 1\n",
    )
    .unwrap();
    let mut report = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let authority = report.authority.as_ref().unwrap();
    assert_eq!(authority["findings"]["total"], 1);
    let semantic_id = authority["findings"]["all"][0]["semantic_id"].clone();
    report.diagnostics.clear();
    let sarif: serde_json::Value =
        serde_json::from_str(&SarifFormatter::new().format_manifest(&report).unwrap()).unwrap();
    assert_eq!(
        sarif["runs"][0]["results"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(
        sarif["runs"][0]["results"][0]["partialFingerprints"]["flowspec.semantic_id"],
        semantic_id
    );
}
