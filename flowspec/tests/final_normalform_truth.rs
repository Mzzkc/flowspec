use flowspec::{analyze, Config, Execution};
#[test]
fn ordinary_python_has_only_input_relevant_debt() {
    for source in ["", "def _unused():\n    return 1\n", "def greet(name):\n    return f'Hello, {name}'\ndef main():\n    print(greet('world'))\nif __name__ == '__main__':\n    main()\n"] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("clean.py"), source).unwrap();
        let report = analyze(root.path(), &Config::default(), &[]).unwrap().manifest;
        assert_eq!(report.execution, Some(Execution::Complete), "{source}: {:?}",report.authority.as_ref().unwrap()["losses"]);
    }
}
#[test]
fn distinct_capability_losses_have_distinct_identity() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("typed.py"), "class A:\n    def value(self, x: int) -> int:\n        if x:\n            return x\n        return 0\n").unwrap();
    let report = analyze(root.path(), &Config::default(), &[])
        .unwrap()
        .manifest;
    let losses = report.authority.as_ref().unwrap()["losses"]
        .as_array()
        .unwrap();
    for (i, a) in losses.iter().enumerate() {
        for b in &losses[i + 1..] {
            assert!(
                a == b || a["id"] != b["id"],
                "distinct losses collide: {a} / {b}"
            );
        }
    }
}
