use assert_cmd::Command;
#[test]
fn packaged_cli_catalog_contains_the_actual_four_language_thirteen_pattern_registry() {
    let out = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["registry"])
        .output()
        .unwrap();
    assert_eq!(
        out.status.code(),
        Some(0),
        "catalog must be available without an analysis target"
    );
    let catalog: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(catalog["languages"].as_array().map(Vec::len), Some(4));
    assert_eq!(catalog["patterns"].as_array().map(Vec::len), Some(13));
    assert_eq!(catalog["languages"][2]["id"], "typescript");
    assert_eq!(
        catalog["languages"][2]["grammar_version"],
        "tree-sitter-javascript/0.25.0"
    );
    assert!(catalog["digest"].as_str().unwrap().starts_with("sha256:"));
}
