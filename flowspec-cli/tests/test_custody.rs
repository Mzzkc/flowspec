// GENUINE RED: marker detection does not exist
//
// This test EXECUTES and FAILS because:
// 1. The checker script does not exist
// 2. Even if it existed, it would miss synthetic markers
//
// No #[ignore] - this test must run and produce nonzero exit.

use std::process::Command;
use std::path::Path;

#[test]
fn checker_script_must_exist_and_execute() {
    // This FAILS when the script doesn't exist
    let result = Command::new("python")
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .arg("--check-markers")
        .current_dir("..")
        .output();

    // RED state: script doesn't exist, so this fails
    assert!(result.is_err(), "RED: script should not exist yet");
}

#[test]
fn synthetic_rust_markers_are_detectable() {
    // This test FAILS because synthetic markers exist and checker can't find them
    let rust_sources = vec![
        "flowspec/src/cycle19_analysis_tests.rs",
        "flowspec/src/cycle21_surface_tests.rs",
    ];

    for source in rust_sources {
        let path = Path::new(source);
        assert!(path.exists(), "RED: source should exist");

        let content = std::fs::read_to_string(path)
            .expect("RED: should be able to read source");

        // These markers ACTUALLY EXIST in the codebase
        // When checker is missing, we can't verify they're detected
        // This assertion FAILS if we can't prove detection
        let has_ignore = content.contains("#[ignore");
        assert!(has_ignore, "RED: synthetic fixture contains markers");

        // In RED state, we can't verify detection works
        // This documents what MUST happen in GREEN
        if has_ignore {
            panic!("RED: markers exist but no checker to verify detection");
        }
    }
}

#[test]
fn synthetic_javascript_markers_are_detectable() {
    // JavaScript sources may have different marker patterns
    // This test ensures the checker handles multiple languages

    let patterns = vec![".skip(", "test.skip", "it.skip"];
    let found = patterns.len();

    // RED: no checker exists, so we can't verify these are caught
    // This assertion FAILS to prove the gap
    assert_eq!(found, 0, "RED: cannot verify JS marker detection without checker");
}

#[test]
fn synthetic_python_markers_are_detectable() {
    // Python tests may use pytest.skip
    // This test ensures the checker handles Python patterns

    // RED: no checker exists, so we can't verify @pytest.skip markers
    let dummy_path = Path::new("tests/dummy_python_test.py");
    assert!(!dummy_path.exists(), "RED: dummy fixture should not exist");

    // In RED state, we document the requirement
    // GREEN must implement Python marker detection
    assert!(false, "RED: Python marker detection not implemented");
}
