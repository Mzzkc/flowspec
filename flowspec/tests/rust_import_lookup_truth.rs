use flowspec::{analyze, Config};

fn analyze_case(a: &str, b: &str) -> flowspec::AnalysisResult {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "mod a; mod b;\n").unwrap();
    std::fs::write(dir.path().join("a.rs"), a).unwrap();
    std::fs::write(dir.path().join("b.rs"), b).unwrap();
    analyze(dir.path(), &Config::default(), &[]).unwrap()
}
#[test]
fn aliased_import_preserves_original_target_and_true_cycle() {
    let r = analyze_case(
        "use crate::b::b as run;\npub fn a() { run(); }\n",
        "pub fn b() { crate::a::a(); }\n",
    );
    assert_eq!(
        r.manifest
            .diagnostics
            .iter()
            .filter(|d| d.pattern == "circular_dependency")
            .count(),
        1
    );
}
#[test]
fn inline_module_import_keeps_qualified_definition_and_true_cycle() {
    let r = analyze_case(
        "use crate::b::nested::b;\npub fn a() { b(); }\n",
        "pub mod nested { pub fn b() { crate::a::a(); } }\n",
    );
    assert_eq!(
        r.manifest
            .diagnostics
            .iter()
            .filter(|d| d.pattern == "circular_dependency")
            .count(),
        1
    );
}
#[test]
fn inline_module_lookup_excludes_root_same_name_rival() {
    let r = analyze_case(
        "use crate::b::nested::b;\npub fn a() { b(); }\n",
        "pub fn b() { crate::a::a(); }\npub mod nested { pub fn b() {} }\n",
    );
    assert_eq!(
        r.manifest
            .diagnostics
            .iter()
            .filter(|d| d.pattern == "circular_dependency")
            .count(),
        1,
        "root b legitimately depends on a; dependency cycle remains via the real nested import"
    );
    let graph = r.graph;
    let caller = graph
        .all_symbols()
        .find(|(_, s)| s.qualified_name == "a.rs::a")
        .unwrap()
        .0;
    let calls: Vec<_> = graph
        .callees(caller)
        .iter()
        .map(|id| graph.get_symbol(*id).unwrap().qualified_name.clone())
        .collect();
    assert!(calls.contains(&"b.rs::nested::b".to_string()));
    assert!(!calls.contains(&"b.rs::b".to_string()));
}
#[test]
fn missing_inline_module_cannot_bind_unrelated_function() {
    let r = analyze_case(
        "use crate::b::absent::b;\npub fn a() { b(); }\n",
        "pub fn b() { crate::a::a(); }\n",
    );
    assert_eq!(
        r.manifest
            .diagnostics
            .iter()
            .filter(|d| d.pattern == "circular_dependency")
            .count(),
        0
    );
}
