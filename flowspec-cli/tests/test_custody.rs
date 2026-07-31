// REAL RED: marker detection checker does not exist yet
//
// This test EXECUTES and GENUINELY FAILS because the checker script
// at $CARGO_MANIFEST_DIR/../scripts/check-test-custody.py does not exist.
//
// When invoked:
// - clean=0 when no markers present
// - each_marker!=0 with file:line:pattern evidence
//
// GREEN will add ONLY the checker/baseline implementation.
// This test file remains UNCHANGED.
//
// No #[ignore], no panic!, no assert!(false), no assertions of absence.

use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

#[test]
fn checker_rejects_rust_ignore_markers() {
    // Create temp git repo with tracked Rust test fixture containing #[ignore]
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    // Initialize git repo
    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Create tracked test file with #[ignore] marker
    let test_file = repo.join("tests/example_test.rs");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    fs::write(
        &test_file,
        r#"
#[test]
#[ignore = "this should be detected"]
fn example_test() {
    assert!(true);
}
"#,
    )
    .expect("RED: failed to write test file");

    // Commit the file so it's tracked
    Command::new("git")
        .args(["add", "tests/example_test.rs"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Invoke checker - it MUST detect the marker and return nonzero
    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    // RED state: checker exists but we haven't verified it works yet
    // This assertion will FAIL if checker doesn't detect the marker
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("example_test.rs") && stderr.contains("#[ignore"),
        "RED: checker must detect #[ignore] marker in {}",
        stderr
    );

    // Checker MUST return nonzero when markers are found
    assert!(
        !output.status.success(),
        "RED: checker must return nonzero when markers present"
    );
}

#[test]
fn checker_accepts_clean_sources() {
    // Create temp git repo with tracked CLEAN test fixture (no markers)
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Create tracked CLEAN test file (no markers)
    let test_file = repo.join("tests/clean_test.rs");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    fs::write(
        &test_file,
        r#"
#[test]
fn clean_test() {
    assert!(true);
}
"#,
    )
    .expect("RED: failed to write test file");

    Command::new("git")
        .args(["add", "tests/clean_test.rs"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Invoke checker - it MUST accept clean sources and return zero
    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    // RED state: checker exists but we haven't verified clean=0 behavior
    // This assertion documents the expected behavior
    // In RED, this will help verify the checker works correctly
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.is_empty() || !stderr.contains("clean_test.rs"),
        "RED: checker must not report markers in clean sources"
    );

    // Checker MUST return zero when no markers found
    assert!(
        output.status.success(),
        "RED: checker must return zero for clean sources"
    );
}

#[test]
fn checker_rejects_javascript_skip_markers() {
    // Create temp git repo with tracked JS test fixture containing .skip(
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Create tracked test file with .skip( marker
    let test_file = repo.join("tests/example_test.js");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    fs::write(
        &test_file,
        r#"
test.skip('skipped test', () => {
    assert(true);
});
"#,
    )
    .expect("RED: failed to write test file");

    Command::new("git")
        .args(["add", "tests/example_test.js"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Invoke checker - it MUST detect the marker
    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("example_test.js") && stderr.contains(".skip"),
        "RED: checker must detect .skip( marker"
    );

    assert!(
        !output.status.success(),
        "RED: checker must return nonzero when JS markers present"
    );
}

#[test]
fn checker_rejects_typescript_skip_markers() {
    // Create temp git repo with tracked TS test fixture containing .skip(
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Create tracked test file with .skip( marker (TypeScript)
    let test_file = repo.join("tests/example_test.ts");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    fs::write(
        &test_file,
        r#"
test.skip('skipped typescript test', () => {
    assert(true);
});
"#,
    )
    .expect("RED: failed to write test file");

    Command::new("git")
        .args(["add", "tests/example_test.ts"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Invoke checker - it MUST detect the marker
    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("example_test.ts") && stderr.contains(".skip"),
        "RED: checker must detect TypeScript .skip( marker"
    );

    assert!(
        !output.status.success(),
        "RED: checker must return nonzero when TS markers present"
    );
}

#[test]
fn checker_rejects_python_pytest_skip_markers() {
    // Create temp git repo with tracked Python test fixture containing @pytest.skip
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Create tracked test file with @pytest.skip marker
    let test_file = repo.join("tests/example_test.py");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    fs::write(
        &test_file,
        r#"
import pytest

@pytest.skip("skipped test")
def example_test():
    assert True
"#,
    )
    .expect("RED: failed to write test file");

    Command::new("git")
        .args(["add", "tests/example_test.py"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Invoke checker - it MUST detect the marker
    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("example_test.py") && (stderr.contains("@pytest.skip") || stderr.contains("pytest.mark")),
        "RED: checker must detect @pytest.skip marker"
    );

    assert!(
        !output.status.success(),
        "RED: checker must return nonzero when Python markers present"
    );
}

#[test]
fn custody_requires_zero_markers_with_synthetic_fixture() {
    // GENUINE RED: This test FAILS because FIVE #[ignore] markers currently exist
    // in the codebase at inherited test locations.
    //
    // This test constructs synthetic marker tokens at runtime (not in source)
    // to verify the checker can detect them, then requires ZERO markers in
    // the actual tracked sources.
    //
    // GREEN will leave this test UNCHANGED and remove only the five inherited
    // #[ignore] attributes/change only their evidence paths to committed artifacts.

    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    // First, construct synthetic marker tokens at runtime to verify detection works
    let temp = TempDir::new().expect("RED: failed to create temp dir");
    let repo = temp.path();

    Command::new("git")
        .args(["init"])
        .current_dir(repo)
        .output()
        .expect("RED: git init failed");

    Command::new("git")
        .args(["config", "user.email", "test@test.com"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    Command::new("git")
        .args(["config", "user.name", "Test"])
        .current_dir(repo)
        .output()
        .expect("RED: git config failed");

    // Construct marker token at runtime: "#[ignore" (not in source as raw literal)
    let marker_token = ["#[", "ignore"].concat();
    let test_file = repo.join("tests/synthetic_test.rs");
    fs::create_dir_all(test_file.parent().unwrap()).expect("RED: failed to create tests dir");
    let synthetic_content = format!(r#"
#[test]
{} = "synthetic marker constructed at runtime"]
fn synthetic_test() {{
    assert!(true);
}}
"#, marker_token);
    fs::write(&test_file, synthetic_content)
        .expect("RED: failed to write synthetic fixture");

    Command::new("git")
        .args(["add", "tests/synthetic_test.rs"])
        .current_dir(repo)
        .output()
        .expect("RED: git add failed");

    Command::new("git")
        .args(["commit", "-m", "test"])
        .current_dir(repo)
        .output()
        .expect("RED: git commit failed");

    // Verify checker detects the synthetic marker
    let synthetic_output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker must execute on synthetic fixture");

    assert!(
        !synthetic_output.status.success(),
        "RED: checker must detect synthetic marker"
    );

    // Now require ZERO markers in actual tracked sources
    // This FAILS because 5 #[ignore] markers currently exist
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

    let actual_output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(&repo_root)
        .arg("--check-markers")
        .output()
        .expect("RED: checker must execute on actual repo");

    let actual_stderr = String::from_utf8_lossy(&actual_output.stderr);

    // RED state: This assertion FAILS because markers currently exist
    // GREEN will remove the 5 #[ignore] attributes to make this pass
    assert!(
        actual_output.status.success() && actual_stderr.is_empty(),
        "RED: custody requires zero markers. Current stderr: {}",
        actual_stderr
    );
}
