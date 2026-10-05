use flowspec::{analyze, commands::expand_graph_tables, Config, Execution};
#[test]
fn disjoint_known_arities_do_not_require_equal_effect_or_unsupported_body() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("entry.py"), "def greet(name):\n    return f'Hello, {name}'\ndef main():\n    print(greet('world'))\nif __name__ == '__main__':\n    main()\n").unwrap();
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    assert_eq!(r.manifest.execution, Some(Execution::Complete), "{:?}", r.manifest.authority);
}
#[test]
fn viable_unsupported_pair_keeps_located_debt() {
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("entry.py"), "def a(x):\n    return f'Hello, {x}'\ndef b(x):\n    return f'Hello, {x}'\n").unwrap();
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    assert_eq!(r.manifest.execution, Some(Execution::Incomplete));
    assert!(r.manifest.authority.as_ref().unwrap()["losses"].as_array().unwrap().iter().any(|l| l["reason"] == "DUP_UNSUPPORTED_CONSTRUCT" && !l["ranges"].as_array().unwrap().is_empty()));
}
#[test]
fn nonempty_full_graph_source_symbols_edges_and_ranges_survive_bounded_wire() {
    let d = tempfile::tempdir().unwrap();
    for i in 0..20 {
        std::fs::write(d.path().join(format!("mod_{i}.py")), format!("import os\nimport sys\ndef func_{i}(x):\n    return os.path.join(str(x), sys.argv[0])\n")).unwrap();
    }
    let r = analyze(d.path(), &Config::default(), &[]).unwrap();
    let g = expand_graph_tables(&r.manifest.authority.as_ref().unwrap()["analysis_graph"]).unwrap();
    let symbols: Vec<_> = r.graph.all_symbols().map(|(_,s)|s).collect();
    assert_eq!(g["entities"].as_array().unwrap().len(), symbols.len());
    for s in symbols {
        assert!(g["entities"].as_array().unwrap().iter().any(|e| e["subject"]["qualified_name"] == s.qualified_name && e["range"]["start_line"] == s.location.line));
    }
    assert_eq!(g["dependency_graph"].as_array().unwrap().len(), r.graph.all_symbols().map(|(id,_)|r.graph.edges_from(id).len()).sum::<usize>());
    assert!(serde_json::to_vec(&r.manifest).unwrap().len() < 20480);
}
