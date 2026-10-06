use flowspec::{analyze, Config, OutputFormatter, SarifFormatter};
use std::fs;
use tempfile::TempDir;
#[test]
fn analyzed_sarif_keeps_full_catalog_and_visible_incomplete_execution() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "def broken(:\n").unwrap();
    let report = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let sarif: serde_json::Value =
        serde_json::from_str(&SarifFormatter::new().format_manifest(&report).unwrap()).unwrap();
    let run = &sarif["runs"][0];
    assert_eq!(
        run["tool"]["driver"]["rules"].as_array().map(Vec::len),
        Some(13)
    );
    assert_eq!(run["invocations"][0]["executionSuccessful"], false);
    // Standard SARIF fields above are read directly; only the custom extension
    // uses the public, versioned lossless record decoder.
    let expanded = flowspec::commands::expand_report_records(&sarif).unwrap();
    assert_eq!(expanded["runs"][0]["tool"], run["tool"]);
    assert_eq!(expanded["runs"][0]["invocations"], run["invocations"]);
    assert_eq!(expanded["runs"][0]["results"], run["results"]);
    let run = &expanded["runs"][0];
    assert_eq!(run["properties"]["flowspec"]["execution"], "incomplete");
    assert_eq!(
        run["properties"]["flowspec"]["provenance"]["registry"]["digest"],
        report.authority.as_ref().unwrap()["provenance"]["registry"]["digest"]
    );
    assert_eq!(
        run["properties"]["flowspec"],
        *report.authority.as_ref().unwrap()
    );
}
