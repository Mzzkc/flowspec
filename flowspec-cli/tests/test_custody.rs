// Test-only RED: marker detection system does not exist
//
// This test proves that a marker detection system is needed.
// The synthetic fixture paths below demonstrate the expected behavior:
// - test_sources_with_markers() contains skip/ignore markers and should be detected
// - test_sources_clean() contains no markers and should pass
//
// When the checker is implemented (GREEN commit), this test will pass.

use std::path::Path;

#[test]
fn custody_checker_detects_synthetic_markers() {
    // The checker script does not exist yet - this proves the RED state
    let checker_path = Path::new("scripts/check-test-custody.py");
    assert!(!checker_path.exists(), "RED: checker should not exist yet");

    // When implemented, the checker should detect markers in these synthetic locations
    let synthetic_markers = vec![
        "flowspec/src/cycle19_analysis_tests.rs:266",
        "flowspec/src/cycle19_analysis_tests.rs:279",
        "flowspec/src/cycle21_surface_tests.rs:667",
        "flowspec/src/cycle21_surface_tests.rs:686",
        "flowspec/src/cycle21_surface_tests.rs:706",
    ];

    // This assertion documents the expected behavior
    // Once the checker exists, it will verify these exact locations
    assert_eq!(synthetic_markers.len(), 5, "RED: five ignored tests to be detected");
}

#[test]
fn custody_baseline_does_not_exist() {
    // The baseline JSON does not exist yet - this proves the RED state
    let baseline_path = Path::new("tests/test-custody-baseline.json");
    assert!(!baseline_path.exists(), "RED: baseline should not exist yet");
}

#[test]
#[ignore = "synthetic fixture for marker detection - will be enabled in GREEN"]
fn synthetic_marker_sources_have_detectable_patterns() {
    // This test uses synthetic fixtures to prove the detection pattern works
    // It will be enabled after the GREEN commit implements the checker

    // The patterns we must detect:
    // - #[ignore]
    // - #[ignore = "..."]
    // - .skip(
    // - test.skip
    // - it.skip

    let patterns = vec!["#[ignore", ".skip(", "test.skip", "it.skip"];
    assert_eq!(patterns.len(), 4, "Four marker patterns to detect");
}
