// Test custody enforcement: no #[ignore] or skip markers allowed in tracked tests
use std::path::Path;
use std::process::Command;

#[test]
fn custody_rejects_the_current_five_ignored_tests() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&script)
        .arg("--root")
        .arg(&root)
        .arg("--check-markers")
        .output()
        .expect("custody checker must execute");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Script must exist and run
    if stderr.contains("can't open file") || stderr.contains("No such file") {
        panic!("RED: check-test-custody.py does not exist yet");
    }

    // And it should report exactly the five known ignored tests
    assert!(stderr.contains("cycle19_analysis_tests.rs:266"));
    assert!(stderr.contains("cycle19_analysis_tests.rs:279"));
    assert!(stderr.contains("cycle21_surface_tests.rs:667"));
    assert!(stderr.contains("cycle21_surface_tests.rs:686"));
    assert!(stderr.contains("cycle21_surface_tests.rs:706"));
}
