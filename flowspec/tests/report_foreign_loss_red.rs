#![cfg(feature = "fault_probe")]
use flowspec::fault_probe::{analyze_with_plan, FaultCase, FaultPlan};
use flowspec::{analyze, Config};
#[test]
fn foreign_selected_loss_is_not_automatically_outside_consumer_scope() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("helpers.py"),
        "def _unused_helper():\n    return 1\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("bridge.js"), "const enabled = true;\n").unwrap();
    let normal = analyze(dir.path(), &Config::default(), &[]).unwrap();
    assert_eq!(normal.manifest.authority.unwrap()["findings"]["total"], 1);
    let plan = FaultPlan::new(
        FaultCase::ReadPermissionError,
        "bridge.js",
        "foreign-closure-control",
    )
    .unwrap();
    let result = analyze_with_plan(dir.path(), &Config::default(), &[], plan).unwrap();
    assert!(!result.operations.is_empty());
    let a = result.analysis.manifest.authority.unwrap();
    assert_eq!(a["execution"], "incomplete");
    assert_eq!(
        a["findings"]["total"], 0,
        "a language label is not interop closure proof"
    );
    assert!(a["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|loss| loss["required_capabilities"]
            .as_array()
            .is_some_and(|items| items.iter().any(|i| i == "consumer_closure"))));
}
