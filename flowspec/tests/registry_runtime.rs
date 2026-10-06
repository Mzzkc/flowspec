use flowspec::parser::ir::EdgeKind;
use flowspec::{analyze, Config, FlowspecError};
use std::fs;
use tempfile::TempDir;

#[test]
fn unsupported_language_names_include_typescript() {
    let error = FlowspecError::UnsupportedLanguage {
        language: "synthetic".into(),
    };
    assert!(
        error.to_string().contains("typescript"),
        "all four registry names must be in the error: {error}"
    );
}

#[test]
fn report_exposes_result_determining_registry_descriptors() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let a = analyze(dir.path(), &Config::default(), &[])
        .unwrap()
        .manifest
        .authority
        .unwrap();
    let registered = flowspec::registry::Registry::default();
    let registry = registered.metadata();
    assert_eq!(
        a["provenance"]["registry"]["digest"],
        serde_json::json!(registered.digest())
    );
    assert_eq!(
        registry["languages"].as_array().map(Vec::len),
        Some(4),
        "public catalog must carry the same language descriptors bound by the report digest"
    );
    assert_eq!(registry["patterns"].as_array().map(Vec::len), Some(13));
    assert_eq!(registry["languages"][2]["id"], "typescript");
    assert_eq!(
        registry["languages"][2]["extensions"],
        serde_json::json!(["ts", "tsx"])
    );
    assert!(registry["languages"][2]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "advanced_type_evaluation_unavailable"));
    assert_eq!(
        registry["languages"][2]["grammar_version"],
        "tree-sitter-typescript/0.23.2+tsx"
    );
    assert!(!registry["languages"][2]["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|v| v == "typescript_preprocessing"));
    assert!(registered.digest().starts_with("sha256:"));
}

#[test]
fn alias_selection_uses_canonical_report_language() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let result = analyze(dir.path(), &Config::default(), &["py".into()]);
    assert!(
        result.is_ok(),
        "registered language alias must select its factory"
    );
    let a = result.unwrap().manifest.authority.unwrap();
    assert_eq!(
        a["scope"]["effective_languages"],
        serde_json::json!(["python"])
    );
}

#[test]
fn invalid_config_language_fails_before_analysis() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.py"), "x = 1\n").unwrap();
    let config = Config {
        languages: vec!["unregistered".into()],
        ..Default::default()
    };
    assert!(matches!(
        analyze(dir.path(), &config, &[]),
        Err(FlowspecError::UnsupportedLanguage { .. })
    ));
}

#[test]
fn python_binding_resolves_python_module_with_foreign_same_stem_rival() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.py"),
        "from utils import target\ndef invoke():\n    return target()\n",
    )
    .unwrap();
    fs::write(dir.path().join("utils.py"), "def target():\n    return 1\n").unwrap();
    fs::write(
        dir.path().join("utils.js"),
        "export function target() { return 2; }\n",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let import = result
        .graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.ends_with("entry.py")
                && s.name == "target"
                && s.annotations.contains(&"import".into())
        })
        .unwrap()
        .0;
    let targets: Vec<_> = result
        .graph
        .edges_from(import)
        .iter()
        .filter(|e| e.kind == EdgeKind::References)
        .filter_map(|e| {
            result
                .graph
                .get_symbol(e.target)
                .map(|s| s.location.file.clone())
        })
        .collect();
    assert_eq!(
        targets,
        vec![dir.path().join("utils.py")],
        "import binding must resolve its language namespace, never a same-name rival"
    );
}

#[test]
fn javascript_and_typescript_selection_remain_disjoint() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("a.js"), "export function jsOnly() {}\n").unwrap();
    fs::write(
        dir.path().join("a.ts"),
        "export function tsOnly(): void {}\n",
    )
    .unwrap();
    for (language, extension) in [("javascript", "js"), ("typescript", "ts")] {
        let result = analyze(dir.path(), &Config::default(), &[language.into()]).unwrap();
        assert!(!result.graph.all_symbols().collect::<Vec<_>>().is_empty());
        assert!(result
            .graph
            .all_symbols()
            .all(|(_, s)| s.location.file.extension().unwrap() == extension));
    }
}
