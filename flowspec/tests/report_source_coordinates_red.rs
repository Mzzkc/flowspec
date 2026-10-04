use flowspec::{analyze, Config};
#[test]
fn actual_graph_module_uses_consumed_file_end_not_internal_sentinel() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("module.py"),
        "def main():\n    return 1\nmain()\n",
    )
    .unwrap();
    let result = analyze(dir.path(), &Config::default(), &[]).unwrap();
    for (_, symbol) in result.graph.all_symbols() {
        assert!(
            symbol.location.end_line <= 4,
            "actual graph location: {:?}",
            symbol.location
        );
    }
}
