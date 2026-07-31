use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CHECKER_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn source_checker_path() -> PathBuf {
    repo_root().join("scripts/check-test-custody.py")
}

fn source_policy_path() -> PathBuf {
    repo_root().join("tests/test-custody-policy.json")
}

fn run_bounded(mut command: Command, timeout: Duration, context: &str) -> Output {
    let mut child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("{context}: failed to spawn subprocess: {error}"));
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .unwrap_or_else(|error| panic!("{context}: failed to poll subprocess: {error}"))
            .is_some()
        {
            return child
                .wait_with_output()
                .unwrap_or_else(|error| panic!("{context}: failed to collect output: {error}"));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("{context}: subprocess exceeded {} ms", timeout.as_millis());
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn git(repo: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.current_dir(repo).args(args);
    let output = run_bounded(command, GIT_TIMEOUT, &format!("git {args:?}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args).stdout)
        .expect("git output must be UTF-8")
        .trim()
        .to_owned()
}

fn write_file(repo: &Path, relative: &str, content: impl AsRef<[u8]>) {
    let path = repo.join(relative);
    fs::create_dir_all(path.parent().expect("fixture path must have a parent"))
        .expect("fixture parent must be created");
    fs::write(&path, content).expect("fixture file must be written");
}

fn copy_checker_and_policy(repo: &Path) {
    write_file(
        repo,
        "scripts/check-test-custody.py",
        fs::read(source_checker_path()).expect("source checker must be readable"),
    );
    write_file(
        repo,
        "tests/test-custody-policy.json",
        fs::read(source_policy_path()).expect("source policy must be readable"),
    );
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "--all"]);
    git(repo, &["commit", "-m", message]);
    git_text(repo, &["rev-parse", "HEAD"])
}

fn init_clean_repo() -> TempDir {
    let temp = TempDir::new().expect("scratch repository must be created");
    let repo = temp.path();
    git(repo, &["init"]);
    git(
        repo,
        &["config", "user.email", "custody-cli@example.invalid"],
    );
    git(repo, &["config", "user.name", "Custody CLI Authority RED"]);
    copy_checker_and_policy(repo);
    write_file(
        repo,
        "tests/clean_authority.rs",
        "#[test]\nfn clean_authority_control() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );
    commit_all(repo, "clean custody CLI authority fixture");
    assert_clean_fixture(repo);
    temp
}

fn assert_clean_fixture(repo: &Path) {
    let status = git_text(repo, &["status", "--porcelain"]);
    assert_eq!(status, "", "scratch repository must start clean");

    let tracked = git_text(repo, &["ls-files"]);
    for required in [
        "scripts/check-test-custody.py",
        "tests/test-custody-policy.json",
        "tests/clean_authority.rs",
    ] {
        assert!(
            tracked.lines().any(|line| line == required),
            "scratch repository must track {required}; tracked={tracked}"
        );
        assert!(
            repo.join(required).exists(),
            "scratch repository must contain copied/control path {required}"
        );
    }

    let policy = fs::read_to_string(repo.join("tests/test-custody-policy.json"))
        .expect("copied policy must be readable");
    assert!(
        policy.contains("\"schema\"") && policy.contains("flowspec.test_custody_policy"),
        "copied policy must be the custody policy fixture"
    );
}

fn run_checker(repo: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("python3");
    command
        .current_dir(repo)
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .args(args);
    run_bounded(command, CHECKER_TIMEOUT, "custody checker")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn assert_argparse_policy_rejection(output: &Output, contract: &str) {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{contract}: unchanged checker must fail at argparse for --policy, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert!(
        stderr(output).contains("unrecognized arguments: --policy"),
        "{contract}: expected exact old argparse evidence, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
}

fn assert_future_policy_required(output: &Output, contract: &str) {
    assert!(
        !output.status.success(),
        "{contract}: expected policy_required failure, observed success stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert!(
        stderr(output).contains("policy_required"),
        "{contract}: missing stable policy_required evidence, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
}

fn assert_future_exactly_one_action(output: &Output, contract: &str) {
    assert!(
        !output.status.success(),
        "{contract}: expected exactly_one_action_required failure, observed success stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert!(
        stderr(output).contains("exactly_one_action_required"),
        "{contract}: missing stable exactly_one_action_required evidence, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
}

fn sha256sum(repo: &Path, relative: &str) -> String {
    let mut command = Command::new("sha256sum");
    command.current_dir(repo).arg(relative);
    let output = run_bounded(command, GIT_TIMEOUT, "sha256sum");
    assert!(
        output.status.success(),
        "sha256sum {relative} failed: stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
    String::from_utf8(output.stdout)
        .expect("sha256sum output must be UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum must print a digest")
        .to_owned()
}

fn write_valid_legacy_baseline(repo: &Path, relative: &str) {
    let head = git_text(repo, &["rev-parse", "HEAD"]);
    let checker_hash = sha256sum(repo, "scripts/check-test-custody.py");
    let clean_hash = sha256sum(repo, "tests/clean_authority.rs");
    let baseline = json!({
        "source_sha": head,
        "tracked_test_paths": [
            "scripts/check-test-custody.py",
            "tests/clean_authority.rs"
        ],
        "files": {
            "scripts/check-test-custody.py": checker_hash,
            "tests/clean_authority.rs": clean_hash
        }
    });
    write_file(
        repo,
        relative,
        serde_json::to_string_pretty(&baseline).expect("baseline JSON must serialize") + "\n",
    );
}

fn marker(parts: &[&str]) -> String {
    parts.concat()
}

#[test]
fn policy_option_is_recognized_for_marker_action() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let output = run_checker(
        repo,
        &[
            "--check-markers",
            "--policy",
            "tests/test-custody-policy.json",
        ],
    );

    assert_argparse_policy_rejection(&output, "marker action explicit --policy RED");
    assert!(
        output.status.success(),
        "RED: unchanged checker rejected explicit --policy before marker action authority executed; stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
}

#[test]
fn marker_action_requires_explicit_policy() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let output = run_checker(repo, &["--check-markers"]);

    if output.status.success() {
        assert_eq!(
            stdout(&output),
            "",
            "old clean marker success must be quiet on stdout"
        );
        assert_eq!(
            stderr(&output),
            "",
            "old clean marker success must be quiet on stderr"
        );
        panic!(
            "RED: unchanged checker accepted --check-markers without explicit policy; expected policy_required"
        );
    }
    assert_future_policy_required(&output, "marker action without policy");
}

#[test]
fn generation_action_requires_explicit_policy() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let baseline = "tests/generated-without-policy.json";
    assert!(
        !repo.join(baseline).exists(),
        "generation destination must start absent"
    );

    let output = run_checker(repo, &["--generate-baseline", baseline]);
    if output.status.success() {
        assert!(
            repo.join(baseline).exists(),
            "old generation success must write the destination baseline"
        );
        let generated = fs::read_to_string(repo.join(baseline))
            .expect("old generated baseline must be readable");
        assert!(
            generated.contains("\"source_sha\"") && generated.contains("\"tracked_test_paths\""),
            "old generated baseline must have legacy baseline shape: {generated}"
        );
        panic!(
            "RED: unchanged checker generated and wrote a baseline without explicit policy; expected policy_required before mutation"
        );
    }

    assert_future_policy_required(&output, "generation without policy");
    assert!(
        !repo.join(baseline).exists(),
        "policy_required generation failure must not create destination"
    );
}

#[test]
fn baseline_action_requires_explicit_policy() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let baseline = "tests/valid-legacy-baseline.json";
    write_valid_legacy_baseline(repo, baseline);

    let output = run_checker(repo, &["--baseline", baseline]);
    if output.status.success() {
        assert_eq!(
            stdout(&output),
            "",
            "old valid baseline success must be quiet on stdout"
        );
        assert_eq!(
            stderr(&output),
            "",
            "old valid baseline success must be quiet on stderr"
        );
        panic!(
            "RED: unchanged checker verified a valid legacy baseline without explicit policy; expected policy_required"
        );
    }

    assert_future_policy_required(&output, "baseline verification without policy");
}

#[test]
fn self_test_action_requires_explicit_policy() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let output = run_checker(repo, &["--self-test"]);

    if output.status.success() {
        assert!(
            stderr(&output).contains("SELF_TEST: lexical fail-closed marker detection active"),
            "old self-test success must emit the self-test banner, stdout={} stderr={}",
            stdout(&output),
            stderr(&output)
        );
        panic!(
            "RED: unchanged checker ran --self-test without explicit policy; expected policy_required"
        );
    }

    assert_future_policy_required(&output, "self-test without policy");
}

#[test]
fn multiple_actions_are_rejected_before_execution() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let rust_ignore = marker(&["#", "[", "ignore", "]"]);
    write_file(
        repo,
        "tests/live_marker.rs",
        format!("{rust_ignore}\n#[test]\nfn live_marker_witness() {{}}\n"),
    );
    commit_all(
        repo,
        "add live marker witness for multiple action selection",
    );
    let generated = "tests/multiple-actions-generated.json";
    assert!(
        !repo.join(generated).exists(),
        "multiple-action generation destination must start absent"
    );

    let output = run_checker(repo, &["--check-markers", "--generate-baseline", generated]);
    if stderr(&output).contains("rust_ignore:") {
        assert!(
            !repo.join(generated).exists(),
            "old branch-order marker scan must not also generate the baseline"
        );
        panic!(
            "RED: unchanged checker accepted two actions and selected --check-markers; expected exactly_one_action_required before scanning"
        );
    }

    assert_future_exactly_one_action(&output, "multiple action invocation");
    assert!(
        !stderr(&output).contains("rust_ignore:"),
        "exactly-one-action failure must occur before marker scanning"
    );
    assert!(
        !repo.join(generated).exists(),
        "exactly-one-action failure must not mutate generation destination"
    );
}

#[test]
fn zero_actions_are_rejected_as_ambiguous() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let output = run_checker(repo, &[]);
    let combined = format!("{}{}", stdout(&output), stderr(&output));

    if !combined.contains("exactly_one_action_required") {
        assert_eq!(
            output.status.code(),
            Some(2),
            "old zero-action path must be argparse/help exit 2, stdout={} stderr={}",
            stdout(&output),
            stderr(&output)
        );
        assert!(
            combined.contains("usage:"),
            "old zero-action path must be generic help-only output, stdout={} stderr={}",
            stdout(&output),
            stderr(&output)
        );
        panic!(
            "RED: unchanged checker treated zero actions as generic help; expected exactly_one_action_required"
        );
    }

    assert_future_exactly_one_action(&output, "zero action invocation");
}
