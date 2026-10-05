use assert_cmd::Command;
use serde_json::Value;
#[test]
fn incomplete_diff_refuses_resolution() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("project");
    std::fs::create_dir(&root).unwrap();
    let before = dir.path().join("before.json");
    let after = dir.path().join("after.json");
    for (source, output) in [
        ("def main():\n    return 1\nmain()\n", &before),
        ("def broken(:\n", &after),
    ] {
        std::fs::write(root.join("main.py"), source).unwrap();
        let result = Command::cargo_bin("flowspec")
            .unwrap()
            .args([
                "analyze",
                root.to_str().unwrap(),
                "--format",
                "json",
                "--quiet",
            ])
            .output()
            .unwrap();
        std::fs::write(output, &result.stdout).unwrap();
    }
    let result = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "diff",
            before.to_str().unwrap(),
            after.to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ])
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(3));
    let value: Value = serde_json::from_slice(&result.stdout).unwrap();
    assert!(value["diagnostics_resolved"].as_array().unwrap().is_empty());
    assert!(value["entities_removed"].as_array().unwrap().is_empty());
}
#[test]
fn pagination_keeps_full_identity_and_catalog() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.py"),
        "def _one():\n    return 1\ndef _two():\n    return 2\n",
    )
    .unwrap();
    let run = |extra: &[&str]| {
        let mut c = Command::cargo_bin("flowspec").unwrap();
        c.args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--quiet",
        ]);
        c.args(extra);
        let r = c.output().unwrap();
        assert!(matches!(r.status.code(), Some(0 | 2 | 3)));
        serde_json::from_slice::<Value>(&r.stdout).unwrap()
    };
    let full = run(&[]);
    let page = run(&["--limit", "1", "--offset", "0"]);
    assert_eq!(
        full["authority"]["report_identity"],
        page["authority"]["report_identity"]
    );
    assert_eq!(full["authority"]["findings"], page["authority"]["findings"]);
    assert_eq!(page["authority"]["continuation"]["shown"], 1);
    assert_eq!(
        page["authority"]["continuation"]["matched_total"],
        full["authority"]["findings"]["total"]
    );
    assert_eq!(page["authority"]["continuation"]["next_cursor"], 1);
}
