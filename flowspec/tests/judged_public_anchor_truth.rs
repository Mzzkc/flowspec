use flowspec::{analyze, Config};
#[test]
fn source_public_anchor_preserves_private_closed_component_rival() {
    for (entry, expected) in [("entry", false), ("_entry", true)] {
        let p = tempfile::tempdir().unwrap();
        std::fs::write(p.path().join("worker.py"), "def _work():\n    return 1\n").unwrap();
        std::fs::write(
            p.path().join("unit.py"),
            format!(
                "from worker import _work as alias\ndef {}():\n    return alias()\n",
                entry
            ),
        )
        .unwrap();
        let r = analyze(p.path(), &Config::default(), &[]).unwrap();
        assert_eq!(
            r.manifest
                .diagnostics
                .iter()
                .any(|d| d.pattern == "isolated_cluster"),
            expected
        );
        if !expected {
            assert_eq!(
                r.manifest.authority.as_ref().unwrap()["execution"],
                "complete"
            );
        }
    }
}
