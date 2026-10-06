use flowspec::parser::ir::*;
use flowspec::{analyze, trace_flows_from, Config, Graph};
use std::path::PathBuf;
fn fixture(main: &str) -> (tempfile::TempDir, flowspec::AnalysisResult) {
    let p = tempfile::tempdir().unwrap();
    std::fs::write(p.path().join("worker.py"), "def _work():\n    return 1\n").unwrap();
    std::fs::write(p.path().join("unit.py"), main).unwrap();
    let r = analyze(p.path(), &Config::default(), &[]).unwrap();
    (p, r)
}
#[test]
fn one_source_invocation_has_one_ordered_path_and_exact_definition_caller() {
    let (_p, r) = fixture("from worker import _work as alias\ndef entry():\n    return alias()\n");
    let id = |name: &str| {
        r.graph
            .all_symbols()
            .find(|(_, s)| s.qualified_name == name)
            .unwrap()
            .0
    };
    let entry = id("unit.py::entry");
    let target = id("worker.py::_work");
    let paths = trace_flows_from(&r.graph, entry, 2);
    assert_eq!(
        paths.len(),
        1,
        "one source call must not fabricate another executable path"
    );
    assert_eq!(
        paths[0]
            .steps
            .iter()
            .map(|s| s.edge_kind)
            .collect::<Vec<_>>(),
        [EdgeKind::Calls, EdgeKind::References]
    );
    assert_eq!(paths[0].steps[1].symbol, target);
    assert_eq!(r.graph.callers(target), vec![entry]);
    assert!(r.manifest.authority.as_ref().unwrap()["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["pattern_id"] != "data_dead_end" && f["pattern_id"] != "isolated_cluster"));
}
#[test]
fn repeated_source_invocations_keep_both_call_sites() {
    let (p, r) = fixture(
        "from worker import _work as alias\ndef main():\n    alias()\n    return alias()\n",
    );
    let entry = r
        .graph
        .all_symbols()
        .find(|(_, s)| s.name == "main")
        .unwrap()
        .0;
    let paths = trace_flows_from(&r.graph, entry, 2);
    assert_eq!(paths.len(), 2);
    assert!(paths.iter().all(|p| p.steps.len() == 2
        && p.steps[0].edge_kind == EdgeKind::Calls
        && p.steps[1].edge_kind == EdgeKind::References));
    let public = flowspec::commands::exact_trace_output(
        p.path(),
        &r,
        "unit.py::main",
        2,
        flowspec::commands::TraceDirection::Forward,
    )
    .unwrap();
    assert_eq!(public.paths.len(), 2);
    assert_eq!(r.manifest.flows.iter().filter(|f| f.entry == "unit.py::main").count(), 2, "normal/library flow records retain both source invocations");
    assert_ne!(public.paths[0].id, public.paths[1].id);
    let proxy = r
        .graph
        .all_symbols()
        .find(|(_, s)| s.annotations.iter().any(|a| a == "import"))
        .unwrap()
        .0;
    let ranges: Vec<_> = r
        .graph
        .edges_to(proxy)
        .iter()
        .filter(|e| e.kind == EdgeKind::Calls)
        .map(|e| {
            r.graph
                .get_reference(e.reference_id.unwrap())
                .unwrap()
                .location
                .clone()
        })
        .collect();
    assert_eq!(ranges.len(), 2);
    assert_ne!(ranges[0], ranges[1]);
}
#[test]
fn import_alone_is_not_callable_consumption_and_read_is_not_execution() {
    for (body, consumed) in [
        ("", false),
        ("def entry():\n    return alias\n", true),
        ("def unused():\n    return alias()\n", true),
        ("def entry(alias):\n    return alias()\n", false),
    ] {
        let (_p, r) = fixture(&format!("from worker import _work as alias\n{body}"));
        let target = r
            .graph
            .all_symbols()
            .find(|(_, s)| s.qualified_name == "worker.py::_work")
            .unwrap()
            .0;
        if !body.contains("alias()") || body.contains("entry(alias)") {
            assert!(r.graph.callers(target).is_empty());
        }
        if body.contains("entry(alias)") {
            assert!(r.graph.consumption_edges_to(target).is_empty());
            assert!(
                !r.manifest.authority.as_ref().unwrap()["losses"]
                    .as_array()
                    .unwrap()
                    .is_empty(),
                "unresolved receiver keeps open consumer closure debt"
            );
            continue;
        }
        let dde = r.manifest.authority.as_ref().unwrap()["findings"]["all"]
            .as_array()
            .unwrap()
            .iter()
            .any(|f| {
                f["pattern_id"] == "data_dead_end"
                    && f["subject"]["qualified_name"] == "worker.py::_work"
            });
        assert_eq!(dde, !consumed, "import/read/shadow rival: {body}");
    }
}
fn sym(name: &str, import: bool) -> Symbol {
    Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Function,
        name: name.into(),
        qualified_name: name.into(),
        visibility: Visibility::Private,
        signature: None,
        location: Location {
            file: PathBuf::from("a.py"),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 2,
        },
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: if import {
            vec!["import".into()]
        } else {
            vec![]
        },
    }
}
fn edge(g: &mut Graph, a: SymbolId, b: SymbolId, k: ReferenceKind) {
    g.add_reference(Reference {
        id: ReferenceId::default(),
        from: a,
        to: b,
        kind: k,
        location: Location {
            file: PathBuf::from("a.py"),
            line: 2,
            column: 1,
            end_line: 2,
            end_column: 3,
        },
        resolution: ResolutionStatus::Resolved,
    });
}
#[test]
fn cyclic_import_aliases_terminate_without_inventing_callers() {
    let mut g = Graph::new();
    let a = g.add_symbol(sym("a", true));
    let b = g.add_symbol(sym("b", true));
    let f = g.add_symbol(sym("f", false));
    edge(&mut g, a, b, ReferenceKind::Import);
    edge(&mut g, b, a, ReferenceKind::Import);
    assert!(g.callers(b).is_empty());
    edge(&mut g, f, a, ReferenceKind::Call);
    assert_eq!(g.callers(b), vec![f]);
    assert!(trace_flows_from(&g, f, 20)[0].is_cyclic);
}
