use flowspec::{analyze, diagnose_report, Config, ReportOptions};
fn subject() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("subject.py"),
        "def _unused():\n    return 1\n",
    )
    .unwrap();
    dir
}
#[test]
fn equal_effective_public_options_have_one_fourteen_field_identity() {
    let dir = subject();
    let a = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let d = diagnose_report(
        dir.path(),
        &Config::default(),
        &[],
        &ReportOptions::default(),
    )
    .unwrap();
    assert_eq!(
        a.authority.as_ref().unwrap()["report_identity"],
        d.authority.as_ref().unwrap()["report_identity"]
    );
}
#[test]
fn public_in_memory_pattern_mirror_retains_all_authority_cells() {
    let dir = subject();
    let d = diagnose_report(
        dir.path(),
        &Config::default(),
        &[],
        &ReportOptions::default(),
    )
    .unwrap();
    assert_eq!(d.pattern_state.len(), 13);
    for p in d.authority.as_ref().unwrap()["pattern_state"]
        .as_array()
        .unwrap()
    {
        for c in p["cells"].as_array().unwrap() {
            assert!(d.pattern_state.contains(&serde_json::json!({"pattern_id":p["pattern_id"],"language":c["language"],"state":c["state"],"reason_codes":c["reason_codes"]})));
        }
    }
}
#[test]
fn compact_wire_reference_rehydrates_truthful_public_mirror() {
    let dir = subject();
    let a = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let json = serde_json::to_vec(&a).unwrap();
    let round: flowspec::manifest::types::Manifest = serde_json::from_slice(&json).unwrap();
    assert_eq!(round.pattern_state, a.pattern_state);
    assert_eq!(round.authority, a.authority);
}
#[test]
fn twenty_file_normal_form_retains_nonempty_graph_ranges_keys_and_public_identity() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..20 {
        std::fs::write(dir.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    let a = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let d = diagnose_report(
        dir.path(),
        &Config::default(),
        &[],
        &ReportOptions::default(),
    )
    .unwrap();
    let before = a.manifest.authority.as_ref().unwrap();
    let after = d.authority.as_ref().unwrap();
    assert_eq!(before, after);
    let graph = flowspec::commands::expand_graph_tables(&before["analysis_graph"]).unwrap();
    assert_eq!(graph["entities"].as_array().unwrap().len(), 60);
    assert_eq!(graph["dependency_graph"].as_array().unwrap().len(), 60);
    assert_eq!(graph["flows"].as_array().unwrap().len(), 20);
    assert!(graph["entities"]
        .as_array()
        .unwrap()
        .iter()
        .all(|e| e["key"].as_str().is_some() && e["range"]["end_byte"].as_u64().is_some()));
    assert_eq!(a.manifest.pattern_state, d.pattern_state);
    assert_eq!(
        serde_json::from_slice::<flowspec::manifest::types::Manifest>(
            &serde_json::to_vec(&d).unwrap()
        )
        .unwrap()
        .pattern_state,
        d.pattern_state
    );
    assert!(serde_json::to_vec(&d).unwrap().len() < 20480);
}
