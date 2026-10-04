use flowspec::{analyze, Config};

#[test]
fn rust_test_registration_consumes_its_function() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("lib.rs"),
        "#[test]\nfn check_case() {}\n\nfn unused_private() {}\n",
    )
    .unwrap();
    let config = Config::load(dir.path(), None).unwrap();
    let report = analyze(dir.path(), &config, &["rust".to_string()]).unwrap();
    let dead: Vec<_> = report
        .manifest
        .diagnostics
        .iter()
        .filter(|finding| finding.pattern == "data_dead_end")
        .map(|finding| finding.entity.as_str())
        .collect();
    assert!(
        dead.iter().any(|name| name.ends_with("unused_private")),
        "{dead:?}"
    );
    assert!(
        dead.iter().all(|name| !name.ends_with("check_case")),
        "{dead:?}"
    );
}
