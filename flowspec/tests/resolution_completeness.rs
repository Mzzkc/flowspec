use flowspec::parser::{
    ir::{ReferenceKind, SymbolKind},
    typescript::TsAdapter,
    LanguageAdapter,
};
use flowspec::{analyze, diagnose_report, Config};
use std::{fs, path::Path};
use tempfile::TempDir;
#[test]
fn type_only_import_and_type_uses_have_annotation_origin() {
    let source="import type { Item } from './model';\nexport function make(value: Item): Item { return value; }";
    let out = TsAdapter::new()
        .parse_output(Path::new("entry.ts"), source)
        .unwrap();
    assert!(out
        .ir
        .symbols
        .iter()
        .any(|s| s.name == "Item" && s.annotations.iter().any(|a| a == "type_only")));
    assert!(out
        .ir
        .references
        .iter()
        .filter(|r| r.location.line == 1)
        .all(|r| r.kind == ReferenceKind::Annotation));
    assert!(out
        .ir
        .references
        .iter()
        .any(|r| r.kind == ReferenceKind::Annotation && r.location.line == 2));
}
#[test]
fn native_implements_produces_original_type_relation() {
    let out = TsAdapter::new()
        .parse_output(
            Path::new("entry.ts"),
            "interface Config {}\nclass App implements Config {}\n",
        )
        .unwrap();
    assert!(out
        .ir
        .symbols
        .iter()
        .any(|s| s.kind == SymbolKind::Interface && s.name == "Config"));
    assert!(out
        .ir
        .references
        .iter()
        .any(|r| r.kind == ReferenceKind::Implement && r.location.line == 2));
}
#[test]
fn typed_import_graph_edges_do_not_become_runtime_origins() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("model.ts"), "export interface Item {}\n").unwrap();
    let path = dir.path().join("entry.ts");
    fs::write(&path,"import type { Item } from './model';\nexport function make(value: Item): Item { return value; }").unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let imported = result
        .graph
        .all_symbols()
        .find(|(_, s)| s.name == "Item" && s.annotations.iter().any(|a| a == "import"))
        .unwrap()
        .0;
    assert!(result.graph.edges_from(imported).iter().all(|e| e
        .reference_id
        .and_then(|id| result.graph.get_reference(id))
        .is_none_or(|r| r.kind == ReferenceKind::Annotation)));
}
#[test]
fn private_positive_and_call_value_export_negatives_survive_all_languages() {
    let controls = [
        (
            "py",
            "def _orphan():\n    return 1\n",
            "def _orphan():\n    return 1\n_orphan()\n",
            "def _orphan():\n    return 1\ncallback = _orphan\n",
            "def api():\n    return 1\n",
        ),
        (
            "js",
            "function orphan() { return 1; }",
            "function orphan() { return 1; } orphan();",
            "function orphan() { return 1; } const callback = orphan;",
            "export function api() { return 1; }",
        ),
        (
            "ts",
            "function orphan(): number { return 1; }",
            "function orphan(): number { return 1; } orphan();",
            "function orphan(): number { return 1; } const callback = orphan;",
            "export function api(): number { return 1; }",
        ),
        (
            "rs",
            "fn orphan() -> i32 { 1 }",
            "fn orphan() -> i32 { 1 } fn main() { orphan(); }",
            "fn orphan() -> i32 { 1 } fn main() { let callback = orphan; }",
            "pub fn api() -> i32 { 1 }",
        ),
    ];
    for (ext, positive, call, value, export) in controls {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(format!("entry.{ext}"));
        fs::write(&path, positive).unwrap();
        let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
        assert!(
            result.manifest.authority.as_ref().unwrap()["findings"]["all"]
                .as_array()
                .unwrap()
                .iter()
                .any(|f| f["pattern_id"] == "data_dead_end"),
            "{ext}: missing private positive"
        );
        for negative in [call, value, export] {
            fs::write(&path, negative).unwrap();
            let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
            assert!(
                !result.manifest.authority.as_ref().unwrap()["findings"]["all"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|f| f["pattern_id"] == "data_dead_end"),
                "{ext}: false finding for {negative}"
            );
        }
    }
}
#[test]
fn same_options_analyze_and_diagnose_share_authority_after_repeat() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.ts"),
        "export function identity<T>(value: T): T { return value; }",
    )
    .unwrap();
    let first = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let warm = analyze(dir.path(), &Config::default(), &[]).unwrap();
    assert_eq!(first.manifest.authority, warm.manifest.authority);
    let diag = diagnose_report(
        dir.path(),
        &Config::default(),
        &[],
        &flowspec::ReportOptions::default(),
    )
    .unwrap();
    assert_eq!(first.manifest.authority, diag.authority);
}

#[test]
fn type_only_unknown_import_does_not_claim_runtime_consumer_loss() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("entry.ts"),
        "import type { Missing } from './absent';\nfunction orphan(): number { return 1; }",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let authority = result.manifest.authority.as_ref().unwrap();
    assert!(authority["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "unresolved_type" && l["origin"] == "Annotation"));
    assert!(authority["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["subject"]["qualified_name"] == "entry.ts::orphan"));
}
#[test]
fn language_scoped_parser_debt_leaves_other_language_type_cell_untouched() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("a.ts"),
        "type F<T> = T extends string ? T : never;",
    )
    .unwrap();
    fs::write(dir.path().join("a.py"), "pass\n").unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    let authority = result.manifest.authority.as_ref().unwrap();
    let pattern = authority["pattern_state"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pattern_id"] == "orphaned_impl")
        .unwrap();
    let py = pattern["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["language"] == "python")
        .unwrap();
    assert!(!py["reason_codes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "unsupported_construct"));
    let ts = pattern["cells"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["language"] == "typescript")
        .unwrap();
    assert!(ts["reason_codes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c == "unsupported_construct"));
}
