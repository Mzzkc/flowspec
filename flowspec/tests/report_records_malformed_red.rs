use flowspec::{analyze, commands::expand_report_records, Config};
#[test]
fn malformed_report_parent_types_return_errors_without_panicking() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("entry.py"), "def _unused():\n    return 1\n").unwrap();
    let wire =
        serde_json::to_value(analyze(d.path(), &Config::default(), &[]).unwrap().manifest).unwrap();
    assert!(wire.get("report_records").is_some());
    for path in [
        vec!["authority"],
        vec!["authority", "inventory"],
        vec!["authority", "findings"],
    ] {
        let mut broken = wire.clone();
        let mut slot = &mut broken;
        for key in path {
            slot = &mut slot[key];
        }
        *slot = serde_json::json!([]);
        assert!(expand_report_records(&broken).is_err());
        assert!(serde_json::from_value::<flowspec::manifest::types::Manifest>(broken).is_err());
    }
}
