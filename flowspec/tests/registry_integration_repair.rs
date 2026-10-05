use flowspec::{analyze, validate_manifest_size, Config, OutputFormatter, SarifFormatter};
use std::path::{Path, PathBuf};
#[test]
fn legacy_collision_projection_preserves_unique_rust_module_bindings() {
    let files: Vec<_> = [
        "project/src/lib.rs",
        "project/src/main.rs",
        "project/src/parser/mod.rs",
        "project/src/parser/rust.rs",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    let map = flowspec::build_module_map(&files);
    let canonical = PathBuf::from("project/src/parser/mod.rs");
    assert_eq!(
        map.values().filter(|path| *path == &canonical).count(),
        1,
        "a unique module binding cannot acquire a competing compatibility alias"
    );
    assert_eq!(map.get("crate::parser"), Some(&canonical));
}
#[test]
fn full_authority_sarif_obeys_preserved_rust_fixture_output_budget() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/rust");
    let result = analyze(&root, &Config::default(), &["rust".into()]).unwrap();
    let output = SarifFormatter::new()
        .format_manifest(&result.manifest)
        .unwrap();
    assert!(
        validate_manifest_size(&output, result.source_bytes, "sarif").is_ok(),
        "full SARIF authority must retain the existing20x/floor budget; bytes={} source={}",
        output.len(),
        result.source_bytes
    );
}
