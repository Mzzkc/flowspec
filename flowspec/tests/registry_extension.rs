use flowspec::parser::{ir::ParseResult, python::PythonAdapter, LanguageAdapter};
use flowspec::registry::{LanguageDescriptor, Registry, PATTERN_IDS};
use flowspec::{analyze, analyze_with_registry, Config, FlowspecError};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
use tempfile::TempDir;
static PARSES: AtomicUsize = AtomicUsize::new(0);
struct Synthetic;
impl LanguageAdapter for Synthetic {
    fn language_name(&self) -> &str {
        "synthetic"
    }
    fn can_handle(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "synth")
    }
    fn parse_file(&self, path: &Path, source: &str) -> Result<ParseResult, FlowspecError> {
        PARSES.fetch_add(1, Ordering::SeqCst);
        PythonAdapter::new().parse_file(path, source)
    }
}
fn factory() -> Box<dyn LanguageAdapter> {
    Box::new(Synthetic)
}
fn descriptor() -> LanguageDescriptor {
    LanguageDescriptor {
        id: "synthetic".into(),
        aliases: vec!["syn".into()],
        extensions: vec!["synth".into()],
        default_enabled: false,
        implementation_id: "test.synthetic.python-ir.v1".into(),
        adapter_version: "1".into(),
        grammar_version: "synthetic-v1".into(),
        capabilities: vec!["symbols".into()],
        limitations: vec!["missing_call_resolution".into()],
        factory: Some(factory),
        recovery_id: "unavailable".into(),
        recovery: None,
        recovery_range: None,
    }
}
#[test]
fn isolated_opt_in_adapter_enters_actual_graph_dispatch_and_report_with_honest_loss() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.synth"),
        "def _orphan():\n    return 1\n",
    )
    .unwrap();
    let mut registry = Registry::default();
    let original = registry.digest();
    registry.register_language(descriptor()).unwrap();
    assert_eq!(Registry::default().languages().len(), 4);
    assert_eq!(
        registry
            .languages()
            .iter()
            .filter(|d| d.default_enabled)
            .count(),
        4
    );
    assert_ne!(registry.digest(), original);
    let default = analyze_with_registry(dir.path(), &Config::default(), &[], &registry).unwrap();
    assert_eq!(
        default.graph.all_symbols().count(),
        0,
        "registration must not make synthetic a default"
    );
    let before = PARSES.load(Ordering::SeqCst);
    let result =
        analyze_with_registry(dir.path(), &Config::default(), &["syn".into()], &registry).unwrap();
    assert!(PARSES.load(Ordering::SeqCst) > before);
    assert!(result.graph.all_symbols().any(|(_, s)| s.name == "_orphan"));
    assert!(
        result
            .manifest
            .diagnostics
            .iter()
            .any(|d| d.pattern == "data_dead_end"),
        "production detector consumes extension IR"
    );
    let a = result.manifest.authority.as_ref().unwrap();
    assert_eq!(
        a["scope"]["effective_languages"],
        serde_json::json!(["synthetic"])
    );
    assert_eq!(a["provenance"]["registry"]["digest"], registry.digest());
    assert_eq!(a["execution"], "incomplete");
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["stage"] == "adapter_capability"));
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["stage"] == "adapter_recovery"));
    let original: Vec<_> = result
        .graph
        .all_symbols()
        .map(|(_, symbol)| symbol)
        .collect();
    assert!(
        !original.is_empty(),
        "factory output must reach the original Graph"
    );
    assert!(original.iter().any(|symbol| symbol.name == "_orphan"));
    assert!(original
        .iter()
        .all(|symbol| symbol.location.file.ends_with("entry.synth")));
    assert!(result
        .manifest
        .entities
        .iter()
        .any(|entity| entity.id.ends_with("::_orphan")));
    let graph = &a["analysis_graph"];
    if graph["projection"]["analysis_graph"] == "omitted" {
        assert_eq!(
            graph["projection"]["compatibility_losses"],
            "/authority/losses"
        );
        assert_eq!(
            graph["projection"]["compatibility_pattern_state"],
            "/authority/pattern_state"
        );
        assert_eq!(a["continuation"]["truncated"], true);
        assert!(a["losses"]
            .as_array()
            .unwrap()
            .iter()
            .any(|loss| loss["reason"] == "output_truncation"));
        assert_eq!(
            result.manifest.execution,
            Some(flowspec::Execution::Incomplete)
        );
    } else {
        let decoded = if graph["encoding"] == "flowspec.graph.tables.v1" {
            flowspec::commands::expand_graph_tables(graph)
                .expect("lossless versioned graph decoder")
        } else {
            graph.clone()
        };
        let entities = decoded["entities"].as_array().unwrap();
        assert_eq!(entities.len(), original.len());
        assert!(!entities.is_empty());
        for symbol in original {
            let entity = entities
                .iter()
                .find(|entity| entity["subject"]["qualified_name"] == symbol.qualified_name)
                .unwrap();
            assert_eq!(entity["subject"]["language"], "synthetic");
            assert!(entity["key"].as_str().unwrap().starts_with("sha256:"));
            assert_eq!(entity["subject"]["binding_id"], entity["key"]);
            assert_eq!(entity["range"]["start_line"], symbol.location.line);
            assert_eq!(entity["range"]["end_line"], symbol.location.end_line);
            assert!(
                entity["range"]["end_byte"].as_u64().unwrap()
                    >= entity["range"]["start_byte"].as_u64().unwrap()
            );
        }
    }
    assert!(analyze(dir.path(), &Config::default(), &["synthetic".into()]).is_err());
}

#[test]
fn registry_rejects_duplicate_id_alias_extension_and_missing_factory_atomically() {
    for mutation in 0..5 {
        let mut r = Registry::default();
        let before = r.digest();
        let mut d = descriptor();
        match mutation {
            0 => d.id = "python".into(),
            1 => d.aliases.push("py".into()),
            2 => d.extensions.push("ts".into()),
            3 => d.factory = None,
            _ => d.default_enabled = true,
        }
        assert!(r.register_language(d).is_err());
        assert_eq!(r.digest(), before);
    }
    let r = Registry::default();
    let mut patterns = r.patterns().to_vec();
    patterns[3].detector = None;
    assert!(Registry::new(r.languages().to_vec(), patterns).is_err());
    let mut patterns = r.patterns().to_vec();
    patterns[3].id = "invented";
    assert!(Registry::new(r.languages().to_vec(), patterns).is_err());
}
#[test]
fn digest_covers_result_determining_language_and_pattern_metadata() {
    let baseline = Registry::default();
    assert_eq!(baseline.digest(), Registry::default().digest());
    for mutation in 0..7 {
        let mut languages = baseline.languages().to_vec();
        let mut patterns = baseline.patterns().to_vec();
        match mutation {
            0 => languages[0].grammar_version.push('x'),
            1 => languages[0].adapter_version.push('x'),
            2 => languages[0].capabilities.push("control_flow".into()),
            3 => languages[0].extensions.push("py2".into()),
            4 => languages[0].implementation_id.push('x'),
            5 => patterns[0].required_capabilities = &["control_flow"],
            _ => patterns[0].implementation_id = "changed-detector-v2",
        }
        assert_ne!(
            Registry::new(languages, patterns).unwrap().digest(),
            baseline.digest()
        );
    }
    assert_eq!(
        baseline.patterns().iter().map(|d| d.id).collect::<Vec<_>>(),
        PATTERN_IDS
    );
}
#[test]
fn relative_javascript_import_uses_exact_binding_path_with_python_rival() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.js"),
        "import { target } from './utils.js'; export function run() { return target(); }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("utils.js"),
        "export function target() { return 2; }\n",
    )
    .unwrap();
    fs::write(dir.path().join("utils.py"), "def target():\n    return 1\n").unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let import = result
        .graph
        .all_symbols()
        .find(|(_, s)| {
            s.location.file.ends_with("entry.js")
                && s.name == "target"
                && s.annotations.contains(&"import".into())
        })
        .unwrap()
        .0;
    let target_paths: Vec<_> = result
        .graph
        .edges_from(import)
        .iter()
        .filter_map(|e| result.graph.get_symbol(e.target))
        .map(|s| s.location.file.clone())
        .collect();
    assert!(target_paths.contains(&dir.path().join("utils.js")));
    assert!(!target_paths.contains(&dir.path().join("utils.py")));
}

#[test]
fn mismatched_callbacks_and_unknown_capabilities_fail_at_registration() {
    let r = Registry::default();
    let mut patterns = r.patterns().to_vec();
    patterns[0].detector = patterns[1].detector;
    assert!(
        Registry::new(r.languages().to_vec(), patterns).is_err(),
        "a registered name must bind its actual detector"
    );
    let mut languages = r.languages().to_vec();
    languages[0].factory = languages[3].factory;
    assert!(
        Registry::new(languages, r.patterns().to_vec()).is_err(),
        "Python cannot silently construct the Rust adapter"
    );
    let mut r = Registry::default();
    let mut d = descriptor();
    d.capabilities.push("invented_guarantee".into());
    assert!(
        r.register_language(d).is_err(),
        "capability declarations use the supported vocabulary"
    );
}

#[test]
fn package_qualified_python_imports_keep_same_name_rivals_separate() {
    let dir = TempDir::new().unwrap();
    for (package, value) in [("one", 1), ("two", 2)] {
        let root = dir.path().join(package);
        fs::create_dir(&root).unwrap();
        fs::write(root.join("pyproject.toml"), "[project]\nname = 'fixture'\n").unwrap();
        fs::write(
            root.join("entry.py"),
            "from utils import target\ndef invoke():\n    return target()\n",
        )
        .unwrap();
        fs::write(
            root.join("utils.py"),
            format!("def target():\n    return {value}\n"),
        )
        .unwrap();
    }
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    for package in ["one", "two"] {
        let source = dir.path().join(package).join("entry.py");
        let import = result
            .graph
            .all_symbols()
            .find(|(_, s)| {
                s.location.file == source
                    && s.name == "target"
                    && s.annotations.contains(&"import".into())
            })
            .unwrap()
            .0;
        let targets: Vec<_> = result
            .graph
            .edges_from(import)
            .iter()
            .filter_map(|e| result.graph.get_symbol(e.target))
            .map(|s| s.location.file.clone())
            .collect();
        assert_eq!(targets, vec![dir.path().join(package).join("utils.py")]);
    }
}
