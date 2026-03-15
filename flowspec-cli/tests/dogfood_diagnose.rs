// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Dogfood tests — run Flowspec diagnostics on the Flowspec repo itself.
//!
//! These tests validate that the diagnostic pipeline doesn't crash on a real
//! Rust codebase and produces valid output format.

use std::path::PathBuf;

/// T15: Dogfood diagnose runs without crash.
///
/// If the diagnostic pipeline crashes on its own codebase, that's a critical
/// reliability failure. Exit 0 = clean, exit 2 = has findings, exit 1 = error.
#[test]
fn test_dogfood_diagnose_does_not_crash() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .args(["diagnose", "."])
        .current_dir(&project_root)
        .output()
        .expect("Failed to run flowspec diagnose");

    let exit_code = output.status.code().unwrap_or(-1);
    assert!(
        exit_code == 0 || exit_code == 2,
        "flowspec diagnose . must exit with 0 (clean) or 2 (findings), not {} (error). \
         stderr: {}",
        exit_code,
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T16: Dogfood diagnose output is parseable YAML.
///
/// Validates the end-to-end pipeline doesn't produce malformed output on a real
/// Rust codebase.
#[test]
fn test_dogfood_diagnose_output_is_parseable() {
    let project_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();

    let output = std::process::Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .args(["diagnose", "."])
        .current_dir(&project_root)
        .output()
        .expect("Failed to run flowspec diagnose");

    let stdout = String::from_utf8_lossy(&output.stdout);
    if !stdout.trim().is_empty() {
        // If there's output, it should be valid YAML
        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
        assert!(
            parsed.is_ok(),
            "Dogfood diagnose output must be valid YAML. Parse error: {:?}\nFirst 500 chars: {}",
            parsed.err(),
            &stdout[..stdout.len().min(500)]
        );
    }
}
