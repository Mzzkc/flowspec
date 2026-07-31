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
    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

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
    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

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
    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

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
    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

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
    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

    let output = Command::new("python")
        .arg(&checker_path)
        .arg("--root")
        .arg(repo)
        .arg("--check-markers")
        .output()
        .expect("RED: checker script does not exist or cannot execute");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("example_test.py")
            && (stderr.contains("@pytest.skip") || stderr.contains("pytest.mark")),
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

    let checker_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py");

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
    let synthetic_content = format!(
        r#"
#[test]
{} = "synthetic marker constructed at runtime"]
fn synthetic_test() {{
    assert!(true);
}}
"#,
        marker_token
    );
    fs::write(&test_file, synthetic_content).expect("RED: failed to write synthetic fixture");

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

fn recovery_checker_path() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py")
}

fn recovery_init_repo() -> TempDir {
    let temp = TempDir::new().expect("failed to create recovery fixture repository");
    let repo = temp.path();

    recovery_git(repo, &["init"]);
    recovery_git(repo, &["config", "user.email", "custody@example.invalid"]);
    recovery_git(repo, &["config", "user.name", "Custody Test"]);
    temp
}

fn recovery_git(repo: &Path, args: &[&str]) -> std::process::Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git must execute in recovery fixture");
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn recovery_git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(recovery_git(repo, args).stdout)
        .expect("git output must be UTF-8")
        .trim()
        .to_owned()
}

fn recovery_write(repo: &Path, relative: &str, content: impl AsRef<[u8]>) {
    let path = repo.join(relative);
    fs::create_dir_all(path.parent().expect("fixture path must have a parent"))
        .expect("fixture parent must be created");
    fs::write(path, content).expect("fixture must be written");
}

fn recovery_commit_all(repo: &Path, message: &str) -> String {
    recovery_git(repo, &["add", "--all"]);
    recovery_git(repo, &["commit", "-m", message]);
    recovery_git_text(repo, &["rev-parse", "HEAD"])
}

fn recovery_run_checker(repo: &Path, args: &[&str]) -> std::process::Output {
    Command::new("python")
        .arg(recovery_checker_path())
        .arg("--root")
        .arg(repo)
        .args(args)
        .output()
        .expect("custody checker must execute")
}

fn recovery_assert_success(output: &std::process::Output, contract: &str) {
    assert!(
        output.status.success(),
        "{contract}: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn recovery_assert_failure_evidence(
    output: &std::process::Output,
    contract: &str,
    evidence: &[&str],
) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "{contract} must fail closed: stdout={stdout} stderr={stderr}"
    );
    for expected in evidence {
        assert!(
            stderr.contains(expected),
            "{contract} missing stable evidence {expected:?}: stderr={stderr}"
        );
    }
}

#[test]
fn checker_scans_every_tracked_test_path_without_exclusions() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let live_attribute = ["#", "[", "ignore", "]"].concat();
    let content = format!("#[test]\n{live_attribute}\nfn excluded_path_probe() {{}}\n");
    let relative = "flowspec-cli/tests/test_custody.rs";

    recovery_write(repo, relative, content);
    recovery_commit_all(repo, "add exact-path custody probe");

    let output = recovery_run_checker(repo, &["--check-markers"]);
    recovery_assert_failure_evidence(
        &output,
        "tracked paths cannot be allowlisted out of custody",
        &[relative, ":2:", "rust_ignore"],
    );
}

#[test]
fn checker_accepts_rust_lexical_controls() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let marker_text = ["#", "[", "ignore", "]"].concat();
    let iterator_call = [".sk", "ip(1)"].concat();
    let content = format!(
        "#[test]\n\
         fn rust_lexical_controls() {{\n\
             let normal = \"first line\n{marker_text}\nlast line\";\n\
             let raw = r#\"\n{marker_text}\n\"#;\n\
             // {marker_text}\n\
             /*\n{marker_text}\n             */\n\
             let character = '#';\n\
             let values = [1, 2, 3];\n\
             let _ = values.iter(){iterator_call}.count();\n\
         }}\n"
    );

    recovery_write(repo, "tests/rust_lexical_controls.rs", content);
    recovery_commit_all(repo, "add Rust lexical controls");

    let output = recovery_run_checker(repo, &["--check-markers"]);
    recovery_assert_success(
        &output,
        "Rust normal/raw strings, comments, chars, and iterators are not markers",
    );
    assert!(
        output.stderr.is_empty(),
        "clean Rust controls must emit no evidence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checker_accepts_javascript_and_typescript_lexical_controls() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let test_prefix = ["test", ".", "skip", "("].concat();
    let it_prefix = ["it", ".", "skip", "("].concat();
    let test_call = format!("{test_prefix}'not live', () => {{}});");
    let it_call = format!("{it_prefix}\"not live\", () => {{}});");

    for extension in ["js", "ts"] {
        let content = format!(
            "// {test_call}\n\
             /*\n{it_call}\n*/\n\
             const doubleQuoted = \"{test_call}\";\n\
             const singleQuoted = '{it_call}';\n\
             const template = `first line\n{test_call}\nlast line`;\n"
        );
        recovery_write(
            repo,
            &format!("tests/lexical_controls.{extension}"),
            content,
        );
    }
    recovery_commit_all(repo, "add JavaScript and TypeScript lexical controls");

    let output = recovery_run_checker(repo, &["--check-markers"]);
    recovery_assert_success(
        &output,
        "JavaScript and TypeScript comments, strings, and templates are not markers",
    );
    assert!(
        output.stderr.is_empty(),
        "clean JavaScript/TypeScript controls must emit no evidence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checker_accepts_python_lexical_controls() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let decorator = ["@", "pytest", ".", "skip"].concat();
    let marked_decorator = ["@", "pytest", ".", "mark", ".", "skip"].concat();
    let content = format!(
        "# {decorator}(\"not live\")\n\
         single = '{decorator}(\"not live\")'\n\
         double = \"{marked_decorator}('not live')\"\n\
         triple = \"\"\"first line\n{marked_decorator}('not live')\nlast line\"\"\"\n"
    );

    recovery_write(repo, "tests/python_lexical_controls.py", content);
    recovery_commit_all(repo, "add Python lexical controls");

    let output = recovery_run_checker(repo, &["--check-markers"]);
    recovery_assert_success(
        &output,
        "Python comments and normal/triple strings are not markers",
    );
    assert!(
        output.stderr.is_empty(),
        "clean Python controls must emit no evidence: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn checker_rejects_live_markers_in_all_supported_test_languages() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let rust_attribute = ["#", "[", "ignore", " = \"reason\"", "]"].concat();
    let javascript_call = ["test", ".", "skip", "('live', () => {});"].concat();
    let typescript_call = ["it", ".", "skip", "('live', () => {});"].concat();
    let python_decorator = ["@", "pytest", ".", "mark", ".", "skip"].concat();

    recovery_write(
        repo,
        "tests/live_rust.rs",
        format!("#[test]\n{rust_attribute}\nfn live_rust() {{}}\n"),
    );
    recovery_write(repo, "tests/live_javascript.js", javascript_call);
    recovery_write(repo, "tests/live_typescript.ts", typescript_call);
    recovery_write(
        repo,
        "tests/live_python.py",
        format!("{python_decorator}\ndef test_live_python():\n    pass\n"),
    );
    recovery_commit_all(repo, "add one live marker per supported language");

    let output = recovery_run_checker(repo, &["--check-markers"]);
    recovery_assert_failure_evidence(
        &output,
        "all supported live marker forms",
        &[
            "tests/live_rust.rs:2:",
            "rust_ignore",
            "tests/live_javascript.js:1:",
            "javascript_test_skip",
            "tests/live_typescript.ts:1:",
            "typescript_test_skip",
            "tests/live_python.py:1:",
            "python_test_skip",
        ],
    );
}

#[cfg(unix)]
#[test]
fn checker_fails_closed_for_tracked_source_loss() {
    use std::os::unix::fs::{symlink, PermissionsExt};

    let temp = recovery_init_repo();
    let repo = temp.path();
    let missing = "tests/missing_test.py";
    let linked = "tests/symlink_test.py";
    let unreadable = "tests/unreadable_test.py";
    let undecodable = "tests/invalid_utf8_test.py";

    recovery_write(repo, missing, "def test_missing():\n    pass\n");
    recovery_write(repo, "symlink-target.py", "def test_target():\n    pass\n");
    fs::create_dir_all(repo.join("tests")).expect("tests directory must exist");
    symlink("../symlink-target.py", repo.join(linked)).expect("tracked symlink must be created");
    recovery_write(repo, unreadable, "def test_unreadable():\n    pass\n");
    recovery_write(
        repo,
        undecodable,
        b"def test_invalid_utf8():\n    value = '\xff'\n",
    );
    recovery_commit_all(repo, "add tracked source-loss fixtures");

    fs::remove_file(repo.join(missing)).expect("tracked source must be removed from worktree");
    fs::set_permissions(repo.join(unreadable), fs::Permissions::from_mode(0o000))
        .expect("tracked source must become unreadable");

    let first = recovery_run_checker(repo, &["--check-markers"]);
    let second = recovery_run_checker(repo, &["--check-markers"]);

    fs::set_permissions(repo.join(unreadable), fs::Permissions::from_mode(0o644))
        .expect("fixture permissions must be restored for cleanup");

    assert_eq!(
        first.stderr, second.stderr,
        "source-loss evidence must be stable across identical runs"
    );
    recovery_assert_failure_evidence(
        &first,
        "tracked source loss",
        &[
            "tests/missing_test.py:0: tracked_source_missing",
            "tests/symlink_test.py:0: tracked_source_symlink",
            "tests/unreadable_test.py:0: source_read_failed",
            "tests/invalid_utf8_test.py:0: invalid_utf8",
        ],
    );
}

#[test]
fn baseline_accepts_an_ancestor_after_a_non_test_commit() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let baseline_temp = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_temp.path().join("baseline.json");

    recovery_write(repo, "tests/stable_test.rs", "#[test]\nfn stable() {}\n");
    let red_parent = recovery_commit_all(repo, "add stable test authority");
    let generate = recovery_run_checker(
        repo,
        &[
            "--generate-baseline",
            baseline.to_str().expect("UTF-8 path"),
        ],
    );
    recovery_assert_success(&generate, "baseline generation");
    assert!(
        fs::read_to_string(&baseline)
            .expect("baseline must be readable")
            .contains(&red_parent),
        "baseline source_sha must name its generating test authority"
    );

    recovery_write(repo, "README.md", "non-test release note\n");
    recovery_commit_all(repo, "add non-test change");

    let verify = recovery_run_checker(
        repo,
        &["--baseline", baseline.to_str().expect("UTF-8 path")],
    );
    recovery_assert_success(
        &verify,
        "an ancestor source_sha with unchanged test paths and bytes remains authoritative",
    );
}

#[test]
fn baseline_rejects_a_non_ancestor_source_authority() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let baseline_temp = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_temp.path().join("baseline.json");

    recovery_write(repo, "tests/stable_test.rs", "#[test]\nfn stable() {}\n");
    let source_sha = recovery_commit_all(repo, "add stable test authority");
    let generate = recovery_run_checker(
        repo,
        &[
            "--generate-baseline",
            baseline.to_str().expect("UTF-8 path"),
        ],
    );
    recovery_assert_success(&generate, "baseline generation");

    let tree = recovery_git_text(repo, &["rev-parse", "HEAD^{tree}"]);
    let unrelated = recovery_git_text(repo, &["commit-tree", &tree, "-m", "unrelated root"]);
    let stored = fs::read_to_string(&baseline).expect("baseline must be readable");
    fs::write(&baseline, stored.replace(&source_sha, &unrelated))
        .expect("baseline authority must be replaced");

    let verify = recovery_run_checker(
        repo,
        &["--baseline", baseline.to_str().expect("UTF-8 path")],
    );
    recovery_assert_failure_evidence(
        &verify,
        "non-ancestor baseline authority",
        &["baseline_source_not_ancestor", &unrelated],
    );
}

#[test]
fn baseline_rejects_test_byte_drift() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let baseline_temp = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_temp.path().join("baseline.json");
    let relative = "tests/byte_drift.rs";

    recovery_write(repo, relative, "#[test]\nfn stable() {}\n");
    recovery_commit_all(repo, "add byte-drift authority");
    let generate = recovery_run_checker(
        repo,
        &[
            "--generate-baseline",
            baseline.to_str().expect("UTF-8 path"),
        ],
    );
    recovery_assert_success(&generate, "baseline generation");

    recovery_write(repo, relative, "#[test]\nfn changed() {}\n");
    let verify = recovery_run_checker(
        repo,
        &["--baseline", baseline.to_str().expect("UTF-8 path")],
    );
    recovery_assert_failure_evidence(&verify, "test byte drift", &[relative]);
}

#[test]
fn baseline_rejects_test_path_drift() {
    let temp = recovery_init_repo();
    let repo = temp.path();
    let baseline_temp = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_temp.path().join("baseline.json");
    let original = "tests/original_path.rs";
    let moved = "tests/moved_path.rs";

    recovery_write(repo, original, "#[test]\nfn stable() {}\n");
    recovery_commit_all(repo, "add path-drift authority");
    let generate = recovery_run_checker(
        repo,
        &[
            "--generate-baseline",
            baseline.to_str().expect("UTF-8 path"),
        ],
    );
    recovery_assert_success(&generate, "baseline generation");

    fs::rename(repo.join(original), repo.join(moved)).expect("test path must move");
    recovery_git(repo, &["add", "--all"]);
    let verify = recovery_run_checker(
        repo,
        &["--baseline", baseline.to_str().expect("UTF-8 path")],
    );
    recovery_assert_failure_evidence(&verify, "test path drift", &[original, moved]);
}

fn recovery_issue_urls(content: &str) -> Vec<String> {
    let prefix = "https://github.com/Mzzkc/flowspec/issues/";
    let mut urls: Vec<String> = content
        .split_whitespace()
        .filter(|token| token.starts_with(prefix))
        .map(str::to_owned)
        .collect();
    urls.sort();
    urls
}

#[test]
fn committed_process_evidence_has_the_exact_issue_partition() {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let cycle_19 =
        fs::read_to_string(repository.join("tests/process-gates/cycle-19/issues-filed.md"))
            .expect("cycle 19 evidence must be readable");
    let cycle_21 =
        fs::read_to_string(repository.join("tests/process-gates/cycle-21/issues-filed.md"))
            .expect("cycle 21 evidence must be readable");

    assert_eq!(
        recovery_issue_urls(&cycle_19),
        [
            "https://github.com/Mzzkc/flowspec/issues/18",
            "https://github.com/Mzzkc/flowspec/issues/19",
            "https://github.com/Mzzkc/flowspec/issues/20",
        ],
        "cycle 19 custody accepts exactly issues 18, 19, and 20"
    );
    assert_eq!(
        recovery_issue_urls(&cycle_21),
        [
            "https://github.com/Mzzkc/flowspec/issues/27",
            "https://github.com/Mzzkc/flowspec/issues/28",
            "https://github.com/Mzzkc/flowspec/issues/29",
        ],
        "cycle 21 custody accepts exactly issues 27, 28, and 29"
    );
}
