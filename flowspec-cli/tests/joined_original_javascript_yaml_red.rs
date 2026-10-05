//! Ordinary requested YAML must retain the selected original JavaScript report.
#[test]
fn original_javascript_requested_yaml_is_a_report_within_the_real_ratio() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/javascript");
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .args([
            "analyze",
            root.to_str().unwrap(),
            "--format",
            "yaml",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert!(
        matches!(output.status.code(), Some(0 | 3)),
        "requested YAML must produce a report: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let raw: serde_json::Value = serde_yaml::from_slice(&output.stdout).unwrap();
    let expanded = flowspec::commands::expand_report_records(&raw).unwrap();
    assert!(expanded["entities"].as_array().unwrap().len() >= 4);
    assert_eq!(expanded["authority"].as_object().unwrap().len(), 17);
}
