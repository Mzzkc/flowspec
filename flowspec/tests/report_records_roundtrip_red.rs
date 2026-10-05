use flowspec::manifest::types::Manifest;
use flowspec::{analyze, Config};
use std::{fs, path::Path};

fn check(root: &Path, bound: usize) {
    let result = analyze(root, &Config::default(), &[]).unwrap();
    assert!(result.graph.all_symbols().count() > 0);
    let original = result.manifest;
    let wire = serde_json::to_vec(&original).unwrap();
    assert!(
        wire.len() < bound,
        "actual complete wire {} >= {}",
        wire.len(),
        bound
    );
    let raw: serde_json::Value = serde_json::from_slice(&wire).unwrap();
    let expanded = flowspec::commands::expand_report_records(&raw).unwrap();
    assert_eq!(&expanded["authority"], original.authority.as_ref().unwrap());
    assert_eq!(
        flowspec::commands::expand_graph_tables(&expanded["authority"]["analysis_graph"]),
        flowspec::commands::expand_graph_tables(
            &original.authority.as_ref().unwrap()["analysis_graph"]
        )
    );
    let round: Manifest = serde_json::from_slice(&wire).unwrap();
    assert_eq!(round.authority, original.authority);
    for field in [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
        "completeness",
    ] {
        // Typed values must survive the actual public serde reader.
        assert_eq!(
            serde_json::to_value(&round).unwrap()[field],
            serde_json::to_value(&original).unwrap()[field]
        );
    }
    assert_eq!(round.entities.len(), original.entities.len());
    assert_eq!(round.diagnostics.len(), original.diagnostics.len());
    assert_eq!(round.pattern_state, original.pattern_state);
    let yaml = flowspec::manifest::OutputFormatter::format_manifest(
        &flowspec::YamlFormatter::new(),
        &original,
    )
    .unwrap();
    assert!(
        yaml.len() < bound,
        "actual full YAML {} >= {}",
        yaml.len(),
        bound
    );
    let legacy = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    let order: Vec<_> = yaml
        .lines()
        .filter_map(|line| line.split_once(':').map(|(key, _)| key))
        .filter(|key| legacy.contains(key))
        .collect();
    assert_eq!(order, legacy);
    let round_yaml: Manifest = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(round_yaml.authority, original.authority);
    assert_eq!(round_yaml.pattern_state, original.pattern_state);
    assert_eq!(round_yaml.entities.len(), original.entities.len());
}

#[test]
fn full_rust_authority_and_legacy_records_fit_and_roundtrip() {
    check(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust"),
        20480,
    );
}

#[test]
fn full_twenty_file_authority_and_legacy_records_fit_and_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(d.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    check(d.path(), 20480);
}

#[test]
fn malformed_record_tables_are_not_contract_valid_reports() {
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("entry.py"), "def _unused():\n    return 1\n").unwrap();
    let result = analyze(d.path(), &Config::default(), &[]).unwrap();
    let raw = serde_json::to_value(result.manifest).unwrap();
    assert!(raw.get("report_records").is_some());
    for kind in 0..4 {
        let mut malformed = raw.clone();
        match kind {
            0 => malformed["report_records"]["encoding"] = serde_json::json!("future-unknown"),
            1 => malformed["authority"]["losses"] = serde_json::json!({"$record":9999999}),
            2 => malformed["report_records"]["nodes"][0] = serde_json::json!(["a", 9999999]),
            _ => malformed["report_records"]["nodes"][0] = serde_json::json!(["o", 0, 0]),
        }
        assert!(flowspec::commands::expand_report_records(&malformed).is_err());
        assert!(serde_json::from_value::<Manifest>(malformed).is_err());
    }
}
