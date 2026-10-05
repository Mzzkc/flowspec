use flowspec::{analyze, commands::compute_diff, Config};
#[test]
fn ordinary_library_diff_never_resolves_from_incomplete_input() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("main.py"),
        "def _unused():\n    return 1\n",
    )
    .unwrap();
    let before = analyze(root.path(), &Config::default(), &[]).unwrap();
    std::fs::write(root.path().join("main.py"), "def broken(:\n").unwrap();
    let after = analyze(root.path(), &Config::default(), &[]).unwrap();
    let diff = compute_diff(&before.manifest, &after.manifest);
    assert!(diff.entities_removed.is_empty());
    assert!(diff.diagnostics_resolved.is_empty());
    assert!(diff.summary.iter().any(|s| s.contains("incomparable")));
}
