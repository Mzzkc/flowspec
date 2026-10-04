// Proposed reachable test-only RED source for flowspec-cli/tests/report_faults_red.rs.
// It uses only the public API at ff25a4e; a baseline failure must be an assertion.
use flowspec::{analyze, Config};
use serde_json::Value;
use std::fs;
use tempfile::TempDir;

fn report(files: &[(&str, &[u8])]) -> Value {
    let root = TempDir::new().expect("fixture root");
    for (name, bytes) in files {
        let path = root.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("fixture parent");
        fs::write(path, bytes).expect("fixture bytes");
    }
    let result = analyze(root.path(), &Config::default(), &[]).expect("analyze");
    serde_json::to_value(&result.manifest).expect("manifest JSON")
}

#[test]
fn fault_red_natural_parse_loss_has_ranged_loss_ledger() {
    let value = report(&[("broken.py", b"def broken(\n")]);
    assert!(
        value["losses"]
            .as_array()
            .is_some_and(|losses| losses.iter().any(|loss| {
                loss["reason"] == "parse_recovery"
                    && loss["scope"]["paths"]
                        .as_array()
                        .is_some_and(|paths| paths.iter().any(|p| p == "broken.py"))
            })),
        "real parse recovery must produce an analyzer-origin ranged loss: {value}"
    );
}

#[test]
fn fault_red_local_import_uncertainty_marks_dependent_pattern() {
    let value = report(&[
        ("pkg/__init__.py", b""),
        ("pkg/use.py", b"from .missing import item\n"),
    ]);
    assert!(
        value["losses"]
            .as_array()
            .is_some_and(|losses| losses.iter().any(|loss| {
                loss["reason"] == "unresolved_import"
                    && loss["scope"]["paths"]
                        .as_array()
                        .is_some_and(|paths| paths.iter().any(|p| p == "pkg/use.py"))
            })),
        "a required local import cannot silently become complete: {value}"
    );
    assert!(
        value["pattern_state"]
            .as_array()
            .is_some_and(|states| states.iter().any(|state| {
                state["state"] == "incomplete"
                    && state["reason_codes"]
                        .as_array()
                        .is_some_and(|codes| codes.iter().any(|code| code == "unresolved_import"))
            })),
        "dependent pattern must carry the unresolved import reason: {value}"
    );
}
