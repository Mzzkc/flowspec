// Test custody enforcement: no #[ignore] or skip markers allowed in tracked tests
use std::path::Path;
use std::process::Command;

#[test]
fn tracked_test_sources_have_no_skip_markers() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&script)
        .arg("--root")
        .arg(&root)
        .arg("--check-markers")
        .output()
        .expect("custody checker must execute");

    // After re-homing process gates to committed evidence, no markers should remain
    assert!(
        output.status.success(),
        "custody checker found skip/ignore markers:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
