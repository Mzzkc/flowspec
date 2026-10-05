use flowspec::parser::{
    ir::ParseResult,
    javascript::JsAdapter,
    python::PythonAdapter,
    report::{Capability, ParseEventCode, ParseOutput},
    rust::RustAdapter,
    typescript::TsAdapter,
    LanguageAdapter,
};
use flowspec::{analyze, Config};
use std::{fs, path::Path};
use tempfile::TempDir;
fn adapters() -> Vec<(
    Box<dyn LanguageAdapter>,
    &'static str,
    &'static str,
    &'static str,
)> {
    vec![
        (
            Box::new(PythonAdapter::new()),
            "a.py",
            "def f():\n    return 1\n",
            "importlib.import_module(name)",
        ),
        (
            Box::new(JsAdapter::new()),
            "a.js",
            "function f() { return 1; }",
            "import(path)",
        ),
        (
            Box::new(TsAdapter::new()),
            "a.ts",
            "function f<T>(value: T): T { return value; }",
            "type F<T> = T extends string ? T : never;",
        ),
        (
            Box::new(RustAdapter::new()),
            "a.rs",
            "fn f() -> i32 { 1 }",
            "include!(path);",
        ),
    ]
}
#[test]
fn genuine_reports_retain_original_source_and_dynamic_sites_for_four_adapters() {
    for (adapter, file, valid, dynamic) in adapters() {
        let clean = adapter.parse_output(Path::new(file), valid).unwrap();
        assert_eq!(clean.report.file, Path::new(file));
        assert_eq!(clean.report.source_bytes, valid.len() as u64);
        assert!(
            clean.report.events.is_empty(),
            "{}: {:?}",
            file,
            clean.report.events
        );
        assert!(clean
            .report
            .complete_for(&[Capability::Symbols, Capability::Calls]));
        assert!(!clean.report.complete_for(&[Capability::ControlFlow]));
        let debt = adapter.parse_output(Path::new(file), dynamic).unwrap();
        let event = debt
            .report
            .events
            .iter()
            .find(|e| {
                matches!(
                    e.code,
                    ParseEventCode::UnsupportedDynamic | ParseEventCode::UnsupportedSyntax
                )
            })
            .unwrap();
        assert!(event.byte_end > event.byte_start);
        assert!(!dynamic[event.byte_start as usize..event.byte_end as usize].is_empty());
        assert!(event.range.column >= 1);
        assert!(!debt
            .report
            .complete_for(&event.affects.iter().copied().collect::<Vec<_>>()));
    }
}
#[test]
fn recovery_is_located_in_original_bytes_for_each_language() {
    for (adapter, file, _, _) in adapters() {
        let content = match file {
            "a.py" => "def (:",
            "a.js" | "a.ts" => "function {",
            _ => "fn (",
        };
        let output = adapter.parse_output(Path::new(file), content).unwrap();
        let event = output
            .report
            .events
            .iter()
            .find(|e| {
                matches!(
                    e.code,
                    ParseEventCode::ParserRecovery | ParseEventCode::MissingNode
                )
            })
            .unwrap();
        assert!(event.byte_end <= content.len() as u64);
        assert!(event.affects.contains(&Capability::Calls));
    }
}
#[test]
fn missing_token_is_distinct_from_error_recovery() {
    let out = RustAdapter::new()
        .parse_output(Path::new("a.rs"), "fn f() { let x = 1 }")
        .unwrap();
    let event = out
        .report
        .events
        .iter()
        .find(|e| e.code == ParseEventCode::MissingNode)
        .unwrap();
    assert_eq!(event.byte_start, event.byte_end);
}
#[test]
fn actual_depth_event_replaces_fake_runtime_read() {
    for (adapter, file, _, _) in adapters() {
        let expr = format!("{}1{}", "(".repeat(300), ")".repeat(300));
        let source = match file {
            "a.py" => format!("def f():\n    return {expr}\n"),
            "a.js" | "a.ts" => format!("function f() {{ return {expr}; }}"),
            _ => format!("fn f() -> i32 {{ {expr} }}"),
        };
        let out = adapter.parse_output(Path::new(file), &source).unwrap();
        let event = out
            .report
            .events
            .iter()
            .find(|e| e.code == ParseEventCode::DepthTruncated)
            .unwrap();
        assert!(event.affects.contains(&Capability::ControlFlow));
        assert!(event.byte_end > event.byte_start);
        assert!(!out
            .ir
            .references
            .iter()
            .any(|r| format!("{:?}", r.resolution).contains("depth_truncation")));
    }
}
#[test]
fn serialized_legacy_ir_cannot_attest_report_completeness() {
    let legacy = serde_json::to_string(&ParseResult::default()).unwrap();
    assert!(serde_json::from_str::<ParseOutput>(&legacy).is_err());
    let ir: ParseResult = serde_json::from_str(&legacy).unwrap();
    let bridged = ParseOutput::from_legacy(Path::new("cached.ts"), "", "typescript", ir);
    assert!(!bridged.report.complete_for(&[Capability::Symbols]));
    assert!(bridged
        .report
        .events
        .iter()
        .any(|e| e.code == ParseEventCode::UnknownLegacyReport));
}
#[test]
fn ordinary_analysis_consumes_structured_dynamic_debt() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.js"),
        "function privateFn() { return 1; }\nimport(path);",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let authority = result.manifest.authority.as_ref().unwrap();
    assert!(authority["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["required_capabilities"]
            .as_array()
            .is_some_and(|c| c.iter().any(|c| c == "consumer_closure"))
            && l["reason"] == "unsupported_construct"));
}

struct LegacyExtension;
impl LanguageAdapter for LegacyExtension {
    fn language_name(&self) -> &str {
        "legacy"
    }
    fn can_handle(&self, path: &Path) -> bool {
        path.extension().is_some_and(|e| e == "legacy")
    }
    fn parse_file(
        &self,
        path: &Path,
        source: &str,
    ) -> Result<ParseResult, flowspec::FlowspecError> {
        PythonAdapter::new().parse_file(path, source)
    }
}
fn legacy_factory() -> Box<dyn LanguageAdapter> {
    Box::new(LegacyExtension)
}
#[test]
fn legacy_extension_cannot_turn_declared_capabilities_into_complete_reports() {
    let mut registry = flowspec::registry::Registry::default();
    registry
        .register_language(flowspec::registry::LanguageDescriptor {
            id: "legacy".into(),
            aliases: vec![],
            extensions: vec!["legacy".into()],
            default_enabled: false,
            implementation_id: "external.legacy.v1".into(),
            adapter_version: "1".into(),
            grammar_version: "legacy".into(),
            capabilities: Capability::ALL.iter().map(|c| c.as_str().into()).collect(),
            limitations: vec![],
            factory: Some(legacy_factory),
            recovery_id: "test-no-errors".into(),
            recovery: Some(|_, _| false),
            recovery_range: None,
        })
        .unwrap();
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.legacy"),
        "def _orphan():\n    return 1\n",
    )
    .unwrap();
    let result = flowspec::analyze_with_registry(
        dir.path(),
        &Config::default(),
        &["legacy".into()],
        &registry,
    )
    .unwrap();
    assert!(result.graph.all_symbols().any(|(_, s)| s.name == "_orphan"));
    let authority = result.manifest.authority.as_ref().unwrap();
    assert_eq!(authority["execution"], "incomplete");
    assert!(authority["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["stage"] == "adapter_recovery"
            && l["required_capabilities"]
                .as_array()
                .is_some_and(|caps| caps.iter().any(|c| c == "consumer_closure"))));
    assert!(authority["pattern_state"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|p| p["pattern_id"] != "layer_violation")
        .all(|p| p["cells"][0]["state"] == "incomplete"));
}
#[test]
fn native_tsx_runtime_accepts_abi_and_original_recovery_free_source() {
    for language in [
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT,
        tree_sitter_typescript::LANGUAGE_TSX,
    ] {
        let grammar: tree_sitter::Language = language.into();
        assert!(
            (tree_sitter::MIN_COMPATIBLE_LANGUAGE_VERSION..=tree_sitter::LANGUAGE_VERSION)
                .contains(&{ grammar.abi_version() })
        );
        let mut parser = tree_sitter::Parser::new();
        parser.set_language(&grammar).unwrap();
    }
    let source =
        "export function View(props: { title: string }) { return <h1>{props.title}</h1>; }";
    let output = TsAdapter::new()
        .parse_output(Path::new("view.tsx"), source)
        .unwrap();
    assert!(!output.report.events.iter().any(|e| matches!(
        e.code,
        ParseEventCode::ParserRecovery | ParseEventCode::MissingNode
    )));
}
#[test]
fn advanced_ts_semantics_have_located_type_and_decorator_debt() {
    for (source, capability) in [
        (
            "type Keys<T> = { [K in keyof T]: T[K] };",
            Capability::Types,
        ),
        ("@register\nclass App {}", Capability::Registration),
    ] {
        let output = TsAdapter::new()
            .parse_output(Path::new("modern.ts"), source)
            .unwrap();
        assert!(output
            .report
            .events
            .iter()
            .any(|e| e.affects.contains(&capability) && e.byte_end > e.byte_start));
        assert!(!output.report.complete_for(&[capability]));
    }
}

#[test]
fn actual_legacy_graph_cache_does_not_replace_current_structured_analysis() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.ts"),
        "function orphan(): number { return 1; }",
    )
    .unwrap();
    let cold = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let cache = dir.path().join(".flowspec/cache");
    flowspec::Graph::new().save(&cache).unwrap();
    fs::write(
        cache.join("legacy-ir.json"),
        serde_json::to_vec(&ParseResult::default()).unwrap(),
    )
    .unwrap();
    let legacy = flowspec::Graph::load(&cache).unwrap();
    assert_eq!(legacy.all_symbols().count(), 0);
    let warm = analyze(dir.path(), &Config::default(), &[]).unwrap();
    assert!(warm.graph.all_symbols().any(|(_, s)| s.name == "orphan"));
    assert_eq!(cold.manifest.authority, warm.manifest.authority);
    assert_eq!(
        warm.manifest.authority.as_ref().unwrap()["provenance"]["cache"]["executed_mode"],
        "full"
    );
}
