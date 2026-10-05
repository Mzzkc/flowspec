use flowspec::{
    analyze, commands::expand_graph_tables, diagnose_report, Config, Execution, ReportOptions,
};
#[test]
fn complete_twenty_file_graph_roundtrips_without_loss_or_catalog_change() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        std::fs::write(dir.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    let original = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let report = diagnose_report(
        dir.path(),
        &Config::default(),
        &[],
        &ReportOptions::default(),
    )
    .unwrap();
    assert_eq!(report.execution, Some(Execution::Complete));
    let before = original.authority.as_ref().unwrap();
    let after = report.authority.as_ref().unwrap();
    assert_eq!(
        expand_graph_tables(&after["analysis_graph"]).unwrap(),
        before["analysis_graph"]
    );
    for field in [
        "findings",
        "scope",
        "inventory",
        "provenance",
        "losses",
        "pattern_state",
        "gate",
    ] {
        assert_eq!(before[field], after[field], "{field}");
    }
    assert_eq!(report.entities.len(), original.entities.len());
    assert!(serde_json::to_vec(&report).unwrap().len() < 20480);
}
#[test]
fn malformed_table_path_refuses_instead_of_panicking() {
    for path in ["f", "é"] {
        let packed = serde_json::json!({"encoding":"flowspec.graph.tables.v1","tables":{"paths":[path],"names":[[true,"x"]]},"entities":[[0,0,0,0,0]]});
        assert!(expand_graph_tables(&packed).is_none());
    }
}
