//! The original selected JavaScript source must fit the requested YAML renderer.
use flowspec::{analyze, manifest::OutputFormatter, Config};
#[test]
fn full_original_javascript_yaml_retains_manifest_within_real_source_ratio() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/javascript");
    let result = analyze(&root, &Config::default(), &["javascript".into()]).unwrap();
    let yaml = flowspec::YamlFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();
    let source_bytes = result.source_bytes;
    assert!(
        yaml.len() <= source_bytes * 10,
        "full original YAML {} exceeds actual selected {} at10x",
        yaml.len(),
        source_bytes
    );
    let round: flowspec::manifest::types::Manifest = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(round.authority, result.manifest.authority);
    assert_eq!(round.entities.len(), result.manifest.entities.len());
}
