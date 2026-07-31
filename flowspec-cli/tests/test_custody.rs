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
fn checker_detects_five_live_ignore_markers() {
    // This test verifies the checker detects the FIVE current #[ignore] markers
    // at known locations that will be removed in the next GREEN commit.
    //
    // Current markers (5 total):
    // - flowspec/src/cycle19_analysis_tests.rs:266
    // - flowspec/src/cycle19_analysis_tests.rs:279
    // - flowspec/src/cycle21_surface_tests.rs:667
    // - flowspec/src/cycle21_surface_tests.rs:686
    // - flowspec/src/cycle21_surface_tests.rs:706
    //
    // After GREEN re-homes these tests with committed evidence,
    // this test will be updated to expect ZERO markers.

    let checker_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../scripts/check-test-custody.py");

    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");

    // Run checker on the actual repository
    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(&repo_root)
        .arg("--check-markers")
        .output()
        .expect("RED: checker must exist and execute");

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Verify all FIVE markers are detected
    assert!(
        stderr.contains("cycle19_analysis_tests.rs:266"),
        "RED: must detect marker at cycle19:266"
    );
    assert!(
        stderr.contains("cycle19_analysis_tests.rs:279"),
        "RED: must detect marker at cycle19:279"
    );
    assert!(
        stderr.contains("cycle21_surface_tests.rs:667"),
        "RED: must detect marker at cycle21:667"
    );
    assert!(
        stderr.contains("cycle21_surface_tests.rs:686"),
        "RED: must detect marker at cycle21:686"
    );
    assert!(
        stderr.contains("cycle21_surface_tests.rs:706"),
        "RED: must detect marker at cycle21:706"
    );

    // Checker MUST return nonzero when 5 markers present
    assert!(
        !output.status.success(),
        "RED: checker must return nonzero when 5 markers present"
    );

    // Document the expected GREEN state
    // After GREEN, these markers will be removed and this test
    // will be updated to expect ZERO markers.
}
