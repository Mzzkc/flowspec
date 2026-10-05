use flowspec::{analyze, Config};
use std::{fs, path::Path};

#[test]
fn declared_rust_module_call_retains_exact_worker_and_excludes_decoy() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "mod worker; mod decoy; pub fn endpoint() -> i32 { worker::inside() }\n").unwrap();
    for name in ["worker", "decoy"] {
        fs::write(dir.path().join(format!("{name}.rs")), "pub(super) fn inside() -> i32 { 7 }\n").unwrap();
    }
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    let find = |name: &str, file: &str| result.graph.all_symbols().find(|(_, s)| s.name == name && s.location.file.ends_with(file)).unwrap().0;
    let endpoint = find("endpoint", "lib.rs");
    let worker = find("inside", "worker.rs");
    let decoy = find("inside", "decoy.rs");
    assert!(result.graph.callees(endpoint).contains(&worker), "actual module consumer edge is missing");
    assert!(!result.graph.callees(endpoint).contains(&decoy));
    assert!(result.graph.unresolved_calls().is_empty(), "resolved call must discharge its located debt");
    assert!(!result.manifest.diagnostics.iter().any(|d| d.pattern == "data_dead_end" && d.entity.contains("worker::inside")));
}

#[test]
fn file_presence_without_rust_module_declaration_is_not_a_binding() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn endpoint() -> i32 { worker::inside() }\n").unwrap();
    fs::write(dir.path().join("worker.rs"), "pub fn inside() -> i32 { 7 }\n").unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    let endpoint = result.graph.all_symbols().find(|(_, s)| s.name == "endpoint").unwrap().0;
    assert!(result.graph.callees(endpoint).is_empty());
    assert!(!result.graph.unresolved_calls().is_empty());
}

#[test]
fn canonical_javascript_full_yaml_retains_authority_inside_original_ratio() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../tests/fixtures/javascript");
    let result = analyze(&root, &Config::default(), &["javascript".into()]).unwrap();
    let original = result.manifest;
    let yaml = flowspec::manifest::OutputFormatter::format_manifest(&flowspec::YamlFormatter::new(), &original).unwrap();
    assert!(yaml.len() <= 4126 * 10, "full canonical YAML {} exceeds original 4126-byte source ratio", yaml.len());
    let decoded: flowspec::manifest::types::Manifest = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(decoded.authority, original.authority);
    assert_eq!(decoded.pattern_state, original.pattern_state);
    assert_eq!(decoded.entities.len(), original.entities.len());
    assert_eq!(decoded.diagnostics.len(), original.diagnostics.len());
}
