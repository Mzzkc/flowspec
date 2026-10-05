use flowspec::{analyze, Config};
use std::{fs, path::Path};

#[test]
fn declared_rust_module_call_retains_exact_worker_and_excludes_decoy() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "mod worker; mod decoy; pub fn endpoint() -> i32 { worker::inside() }\n",
    )
    .unwrap();
    for name in ["worker", "decoy"] {
        fs::write(
            dir.path().join(format!("{name}.rs")),
            "pub(super) fn inside() -> i32 { 7 }\n",
        )
        .unwrap();
    }
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    let find = |name: &str, file: &str| {
        result
            .graph
            .all_symbols()
            .find(|(_, s)| s.name == name && s.location.file.ends_with(file))
            .unwrap()
            .0
    };
    let endpoint = find("endpoint", "lib.rs");
    let worker = find("inside", "worker.rs");
    let decoy = find("inside", "decoy.rs");
    assert!(
        result.graph.callees(endpoint).contains(&worker),
        "actual module consumer edge is missing"
    );
    assert!(!result.graph.callees(endpoint).contains(&decoy));
    assert!(
        result.graph.unresolved_calls().is_empty(),
        "resolved call must discharge its located debt"
    );
    assert!(!result
        .manifest
        .diagnostics
        .iter()
        .any(|d| d.pattern == "data_dead_end"
            && d.entity == result.graph.get_symbol(worker).unwrap().qualified_name));
}

#[test]
fn file_presence_without_rust_module_declaration_is_not_a_binding() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("lib.rs"),
        "pub fn endpoint() -> i32 { worker::inside() }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("worker.rs"),
        "pub fn inside() -> i32 { 7 }\n",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".into()]).unwrap();
    let endpoint = result
        .graph
        .all_symbols()
        .find(|(_, s)| s.name == "endpoint")
        .unwrap()
        .0;
    assert!(result.graph.callees(endpoint).is_empty());
    assert!(!result.graph.unresolved_calls().is_empty());
}

#[test]
fn canonical_javascript_full_yaml_retains_authority_inside_original_ratio() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/javascript")
        .canonicalize()
        .unwrap();
    let result = analyze(&root, &Config::default(), &[]).unwrap();
    let original = result.manifest;
    let yaml = flowspec::manifest::OutputFormatter::format_manifest(
        &flowspec::YamlFormatter::new(),
        &original,
    )
    .unwrap();
    assert!(
        yaml.len() <= 4126 * 10,
        "full canonical YAML {} exceeds original 4126-byte source ratio",
        yaml.len()
    );
    let decoded: flowspec::manifest::types::Manifest = serde_yaml::from_str(&yaml).unwrap();
    assert_eq!(decoded.authority, original.authority);
    assert_eq!(decoded.pattern_state, original.pattern_state);
    assert_eq!(decoded.entities.len(), original.entities.len());
    assert_eq!(decoded.diagnostics.len(), original.diagnostics.len());
}

#[test]
fn retained_v1_record_reader_does_not_require_a_graph_record_wrapper() {
    let old = serde_json::json!({
        "authority": {"losses": [], "pattern_state": [], "provenance": [],
            "analysis_graph": {"encoding":"flowspec.graph.tables.v1"},
            "inventory":{"entries":[]}, "findings":{"all":[]}},
        "diagnostics":[], "entities":[], "summary":{"$record":0}, "flows":[],
        "type_flows":[], "boundaries":[], "dependency_graph":[],
        "report_records":{"encoding":"flowspec.report.records.v1","shapes":[],"nodes":["retained-summary"]}
    });
    let expanded = flowspec::commands::expand_report_records(&old).unwrap();
    assert_eq!(expanded["summary"], "retained-summary");
    assert_eq!(
        expanded["authority"]["analysis_graph"],
        old["authority"]["analysis_graph"]
    );
}

#[test]
fn v2_string_escape_and_digest_are_reversible_and_malformed_forms_refuse() {
    let digest = "a".repeat(64);
    let report = serde_json::json!({
        "authority":{"losses":[],"pattern_state":[],"provenance":[],
            "inventory":{"entries":[]},"findings":{"all":[]}},
        "diagnostics":[],"entities":[],"summary":{"$record":2},"flows":[],
        "type_flows":[],"boundaries":[],"dependency_graph":[],
        "report_records":{"encoding":"flowspec.report.records.v2","shapes":[],
            "nodes":["~~literal",format!("~{digest}"),[-1,0,1]]}
    });
    let expanded = flowspec::commands::expand_report_records(&report).unwrap();
    assert_eq!(
        expanded["summary"],
        serde_json::json!(["~literal", format!("sha256:{digest}")])
    );
    let mut invalid = report;
    invalid["report_records"]["nodes"][0] = serde_json::json!("~unescaped");
    assert!(flowspec::commands::expand_report_records(&invalid).is_err());
}
