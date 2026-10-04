#![cfg(feature = "fault_probe")]
//! A real lost consuming file cannot establish private absence.
use flowspec::fault_probe::{analyze_with_plan, FaultCase, FaultPlan};
use flowspec::{analyze, Config};
use serde_json::Value;
use std::fs;

fn finding_count(report: &Value) -> usize {
    report["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["subject"]["qualified_name"] == "helpers.py::_unused_helper")
        .count()
}

#[test]
fn lost_real_caller_does_not_create_a_dead_end() {
    let root = tempfile::tempdir().unwrap();
    fs::write(
        root.path().join("helpers.py"),
        "def _unused_helper():\n    return 1\n",
    )
    .unwrap();
    fs::write(
        root.path().join("main.py"),
        "from helpers import _unused_helper\n_unused_helper()\n",
    )
    .unwrap();
    let normal = analyze(root.path(), &Config::default(), &[]).unwrap();
    let normal = serde_json::to_value(normal.manifest.authority.unwrap()).unwrap();
    assert_eq!(finding_count(&normal), 0);
    for case in [FaultCase::ReadPermissionError, FaultCase::WalkerError] {
        let plan = FaultPlan::new(case, "main.py", "closure-red-nonce").unwrap();
        let result = analyze_with_plan(root.path(), &Config::default(), &[], plan).unwrap();
        assert!(
            !result.operations.is_empty(),
            "real fault operation must fire"
        );
        let authority = result.analysis.manifest.authority.unwrap();
        assert_eq!(authority["execution"], "incomplete");
        assert_eq!(
            finding_count(&authority),
            0,
            "lost caller cannot prove no caller"
        );
        assert!(authority["losses"].as_array().unwrap().iter().any(|loss| {
            loss["reason"] == "semantic_proof_unavailable"
                && loss["required_capabilities"]
                    .as_array()
                    .is_some_and(|items| items.iter().any(|item| item == "consumer_closure"))
        }));
    }
}
