use flowspec::{analyze, commands::expand_graph_tables, Config, Execution};
#[test]
fn disjoint_known_arities_do_not_require_equal_effect_or_unsupported_body() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("entry.py"), "def greet(name):\n    return f'Hello, {name}'\ndef main():\n    print(greet('world'))\nif __name__ == '__main__':\n    main()\n").unwrap();
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    assert_eq!(
        r.manifest.execution,
        Some(Execution::Complete),
        "{:?}",
        r.manifest.authority
    );
}
#[test]
fn viable_unsupported_pair_keeps_located_debt() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(
        d.path().join("entry.py"),
        "def a(x):\n    return f'Hello, {x}'\ndef b(x):\n    return f'Hello, {x}'\n",
    )
    .unwrap();
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    assert_eq!(r.manifest.execution, Some(Execution::Incomplete));
    assert!(r.manifest.authority.as_ref().unwrap()["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|l| l["reason"] == "DUP_UNSUPPORTED_CONSTRUCT"
            && !l["ranges"].as_array().unwrap().is_empty()));
}
#[test]
fn nonempty_full_graph_source_symbols_edges_and_ranges_survive_bounded_wire() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..20 {
        std::fs::write(d.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    let g = expand_graph_tables(&r.manifest.authority.as_ref().unwrap()["analysis_graph"]).unwrap();
    let symbols: Vec<_> = r.graph.all_symbols().map(|(_, s)| s).collect();
    assert_eq!(g["entities"].as_array().unwrap().len(), symbols.len());
    for s in symbols {
        assert!(g["entities"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["subject"]["qualified_name"] == s.qualified_name
                && e["range"]["start_line"] == s.location.line));
    }
    let graph = &r.graph;
    let actual_edges: std::collections::BTreeSet<_> = graph
        .all_symbols()
        .flat_map(|(id, s)| {
            graph.edges_from(id).into_iter().filter_map(move |e| {
                graph.get_symbol(e.target).map(|target| {
                    (
                        s.qualified_name.clone(),
                        target.qualified_name.clone(),
                        format!("{:?}", e.kind).to_lowercase(),
                    )
                })
            })
        })
        .collect();
    let decoded_edges: std::collections::BTreeSet<_> = g["dependency_graph"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| {
            let name = |key: &serde_json::Value| {
                g["entities"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|n| n["key"] == *key)
                    .unwrap()["subject"]["qualified_name"]
                    .as_str()
                    .unwrap()
                    .to_string()
            };
            (
                name(&e["source_key"]),
                name(&e["target_key"]),
                e["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(decoded_edges, actual_edges);
    let decoded_calls: std::collections::BTreeSet<_> = g["flows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|flow| {
            assert_eq!(flow["node_keys"].as_array().unwrap().len(), 2);
            assert_eq!(flow["edge_keys"].as_array().unwrap().len(), 1);
            let edge = g["dependency_graph"]
                .as_array()
                .unwrap()
                .iter()
                .find(|e| e["key"] == flow["edge_keys"][0])
                .unwrap();
            assert_eq!(flow["node_keys"][0], edge["source_key"]);
            assert_eq!(flow["node_keys"][1], edge["target_key"]);
            flow["edge_keys"][0].as_str().unwrap().to_string()
        })
        .collect();
    let call_keys: std::collections::BTreeSet<_> = g["dependency_graph"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"] == "calls")
        .map(|e| e["key"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(decoded_calls, call_keys);
    assert_eq!(
        g["type_flows"],
        serde_json::to_value(&r.manifest.type_flows).unwrap()
    );
    assert!(serde_json::to_vec(&r.manifest).unwrap().len() < 20480);
}

#[test]
fn lossless_v2_public_reader_reconstructs_source_ranges() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("entry.py"), "def _unused():\n    return 1\n").unwrap();
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    let packed = &r.manifest.authority.as_ref().unwrap()["analysis_graph"];
    assert_eq!(packed["encoding"], "flowspec.graph.tables.v2");
    let decoded = expand_graph_tables(packed).unwrap();
    for (_, s) in r.graph.all_symbols() {
        let e = decoded["entities"]
            .as_array()
            .unwrap()
            .iter()
            .find(|e| e["subject"]["qualified_name"] == s.qualified_name)
            .unwrap();
        assert_eq!(e["range"]["start_line"], s.location.line);
        assert_eq!(e["range"]["start_column"], s.location.column);
        assert_eq!(e["range"]["end_line"], s.location.end_line);
        assert_eq!(e["range"]["end_column"], s.location.end_column);
    }
}
