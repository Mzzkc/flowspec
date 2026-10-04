use flowspec::{analyze, Config};
#[test]
fn natural_adapter_depth_loss_withholds_project_absence() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("helpers.py"),
        "def _helper():\n    return 1\n",
    )
    .unwrap();
    let source = format!(
        "function main(){{{}return 1;{}}}",
        "if(true){".repeat(150),
        "}".repeat(150)
    );
    std::fs::write(dir.path().join("deep.js"), source).unwrap();
    let report = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest
        .authority
        .unwrap();
    assert_eq!(report["execution"], "incomplete");
    assert!(report["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "depth_truncation" && !l["ranges"].as_array().unwrap().is_empty()));
    assert_eq!(report["findings"]["total"], 0);
}
