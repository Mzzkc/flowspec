use flowspec::{analyze, Config};
use flowspec::manifest::types::Manifest;
use std::{fs, path::Path};

fn check(root: &Path, bound: usize) {
    let result = analyze(root, &Config::default(), &[]).unwrap();
    assert!(result.graph.all_symbols().count() > 0);
    let original = result.manifest;
    let wire = serde_json::to_vec(&original).unwrap();
    assert!(wire.len() < bound, "actual complete wire {} >= {}", wire.len(), bound);
    let round: Manifest = serde_json::from_slice(&wire).unwrap();
    assert_eq!(round.authority, original.authority);
    for field in ["metadata", "summary", "diagnostics", "entities", "flows", "boundaries", "dependency_graph", "type_flows", "completeness"] {
        // Typed values must survive the actual public serde reader.
        assert_eq!(serde_json::to_value(&round).unwrap()[field], serde_json::to_value(&original).unwrap()[field]);
    }
    assert_eq!(round.entities.len(), original.entities.len());
    assert_eq!(round.diagnostics.len(), original.diagnostics.len());
    assert_eq!(round.pattern_state, original.pattern_state);
    let yaml = serde_yaml::to_string(&original).unwrap();
    let round_yaml: Manifest = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(round_yaml.authority, original.authority);
    assert_eq!(round_yaml.pattern_state, original.pattern_state);
    assert_eq!(round_yaml.entities.len(), original.entities.len());
}

#[test]
fn full_rust_authority_and_legacy_records_fit_and_roundtrip() {
    check(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/rust"), 20480);
}

#[test]
fn full_twenty_file_authority_and_legacy_records_fit_and_roundtrip() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..20 {
        fs::write(d.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    check(d.path(), 20480);
}
