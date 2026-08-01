use std::fs;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CHECKER_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLICY_SHA256: &str = "c14829a4d7428166cbd9b6a74cff24fb884d52733b549459f0a8ea3c61acc0d1";

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

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn exit_code(output: &Output) -> i32 {
    output.status.code().unwrap_or(-1)
}

fn write_file(repo: &Path, relative: &str, content: impl AsRef<[u8]>) {
    let path = repo.join(relative);
    fs::create_dir_all(path.parent().expect("fixture path must have a parent"))
        .expect("fixture parent must be created");
    fs::write(&path, content).expect("fixture file must be written");
}

fn git(repo: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.current_dir(repo).args(args);
    let output = run_bounded(command, GIT_TIMEOUT, &format!("git {args:?}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
    output
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args).stdout)
        .expect("git output must be UTF-8")
        .trim()
        .to_owned()
}

fn sha256_file(path: &Path) -> String {
    let mut command = Command::new("sha256sum");
    command.arg(path);
    let output = run_bounded(command, GIT_TIMEOUT, "sha256sum");
    assert!(
        output.status.success(),
        "sha256sum {} failed: stdout={} stderr={}",
        path.display(),
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

fn commit_all(repo: &Path, message: &str) {
    git(repo, &["add", "--all"]);
    git(repo, &["commit", "-m", message]);
}

fn init_clean_repo() -> TempDir {
    let temp = TempDir::new().expect("scratch repository must be created");
    let repo = temp.path();
    git(repo, &["init"]);
    git(
        repo,
        &["config", "user.email", "custody-policy@example.invalid"],
    );
    git(repo, &["config", "user.name", "Custody Policy RED"]);

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
    write_file(
        repo,
        "tests/clean_authority.rs",
        "#[test]\nfn clean_authority_control() {\n    assert_eq!(2 + 2, 4);\n}\n",
    );

    commit_all(repo, "clean custody policy fixture");
    assert_clean_fixture(repo);
    temp
}

fn assert_clean_fixture(repo: &Path) {
    let status = git_text(repo, &["status", "--porcelain"]);
    assert_eq!(status, "", "scratch repository must start clean");
    assert_eq!(
        sha256_file(&repo.join("tests/test-custody-policy.json")),
        POLICY_SHA256,
        "scratch policy must be byte-identical to committed authority"
    );

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
    }
}

fn run_checker(repo: &Path, policy: &str, action: &[&str]) -> Output {
    let mut command = Command::new("python3");
    command
        .current_dir(repo)
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .arg("--policy")
        .arg(policy)
        .args(action);
    run_bounded(command, CHECKER_TIMEOUT, "custody checker")
}

fn run_checker_from(caller: &Path, checker: &Path, root: &Path, policy: &str) -> Output {
    let mut command = Command::new("python3");
    command
        .current_dir(caller)
        .arg(checker)
        .arg("--root")
        .arg(root)
        .arg("--policy")
        .arg(policy)
        .arg("--check-markers");
    run_bounded(
        command,
        CHECKER_TIMEOUT,
        "custody checker from alternate caller",
    )
}

fn assert_valid_control(identity: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        0,
        "{identity}: valid policy control must exit 0 before mutation, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stdout(output),
        "",
        "{identity}: valid policy control must keep stdout empty"
    );
    assert_eq!(
        stderr(output),
        "",
        "{identity}: valid policy control must keep stderr empty"
    );
}

fn print_witness(identity: &str, control: &Output, mutation: &str, reason: &str, invalid: &Output) {
    println!(
        "RED_WITNESS identity={identity} control_exit={} mutation={mutation} expected_reason={reason} observed_exit={}",
        exit_code(control),
        exit_code(invalid)
    );
}

fn assert_invalid_policy(identity: &str, reason: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        1,
        "{identity}: invalid policy must exit 1 with {reason}, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stdout(output),
        "",
        "{identity}: invalid policy failure must keep stdout empty"
    );
    let err = stderr(output);
    assert_eq!(
        err.trim(),
        format!("ERROR: {reason}"),
        "{identity}: invalid policy failure must report exactly the stable reason"
    );
    assert!(
        !err.contains("Traceback")
            && !err.contains("rust_ignore")
            && !err.contains("javascript_test_skip")
            && !err.contains("typescript_test_skip")
            && !err.contains("python_test_skip")
            && !err.contains("fatal:")
            && !err.contains("not a git repository")
            && !err.contains("exactly_one_action_required")
            && !err.contains("policy_required"),
        "{identity}: invalid policy failure must not leak traceback, marker, Git, or action diagnostics: {err}"
    );
}

fn run_transport_case(
    identity: &str,
    mutation: &str,
    reason: &str,
    policy: &str,
    mutate: impl FnOnce(&Path),
) {
    let temp = init_clean_repo();
    let repo = temp.path();
    let control = run_checker(repo, "tests/test-custody-policy.json", &["--check-markers"]);
    assert_valid_control(identity, &control);

    mutate(repo);
    let invalid = run_checker(repo, policy, &["--check-markers"]);
    print_witness(identity, &control, mutation, reason, &invalid);
    assert_invalid_policy(identity, reason, &invalid);
}

fn write_repeated(repo: &Path, relative: &str, prefix: &str, repeated: &str, suffix: &str) {
    let mut content = String::with_capacity(prefix.len() + repeated.len() + suffix.len());
    content.push_str(prefix);
    content.push_str(repeated);
    content.push_str(suffix);
    write_file(repo, relative, content);
}

fn deeply_nested_policy(depth: usize) -> String {
    let mut policy = String::new();
    for _ in 0..depth {
        policy.push_str("{\"a\":");
    }
    policy.push('0');
    for _ in 0..depth {
        policy.push('}');
    }
    policy.push('\n');
    policy
}

#[test]
fn policy_missing_path_is_rejected() {
    run_transport_case(
        "policy_missing_path_is_rejected",
        "missing_path",
        "policy_not_found",
        "tests/missing-policy.json",
        |_| {},
    );
}

#[test]
fn policy_directory_is_rejected() {
    run_transport_case(
        "policy_directory_is_rejected",
        "directory",
        "policy_non_regular",
        "tests/policy-directory.json",
        |repo| {
            fs::create_dir_all(repo.join("tests/policy-directory.json"))
                .expect("policy directory must be created");
        },
    );
}

#[test]
fn policy_final_symlink_is_rejected() {
    run_transport_case(
        "policy_final_symlink_is_rejected",
        "final_symlink",
        "policy_symlink_rejected",
        "tests/final-policy-link.json",
        |repo| {
            std::os::unix::fs::symlink(
                "test-custody-policy.json",
                repo.join("tests/final-policy-link.json"),
            )
            .expect("final policy symlink must be created");
        },
    );
}

#[test]
fn policy_parent_symlink_component_is_rejected() {
    run_transport_case(
        "policy_parent_symlink_component_is_rejected",
        "parent_symlink_component",
        "policy_symlink_component_rejected",
        "tests/policy-link-dir/policy.json",
        |repo| {
            write_file(
                repo,
                "alternate-policies/policy.json",
                fs::read(repo.join("tests/test-custody-policy.json"))
                    .expect("committed policy must be readable"),
            );
            std::os::unix::fs::symlink("../alternate-policies", repo.join("tests/policy-link-dir"))
                .expect("parent policy symlink must be created");
        },
    );
}

#[test]
fn policy_fifo_is_rejected_without_blocking() {
    run_transport_case(
        "policy_fifo_is_rejected_without_blocking",
        "fifo",
        "policy_non_regular_fifo",
        "tests/policy-fifo.json",
        |repo| {
            let mut command = Command::new("mkfifo");
            command.current_dir(repo).arg("tests/policy-fifo.json");
            let output = run_bounded(command, CHECKER_TIMEOUT, "mkfifo policy fixture");
            assert!(
                output.status.success(),
                "mkfifo setup failed: stdout={} stderr={}",
                stdout(&output),
                stderr(&output)
            );
        },
    );
}

#[test]
fn policy_socket_is_rejected_without_blocking() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let control = run_checker(repo, "tests/test-custody-policy.json", &["--check-markers"]);
    assert_valid_control("policy_socket_is_rejected_without_blocking", &control);

    let socket_path = repo.join("tests/policy-socket.json");
    let _listener =
        UnixListener::bind(&socket_path).expect("policy socket fixture must be created");
    let invalid = run_checker(repo, "tests/policy-socket.json", &["--check-markers"]);
    print_witness(
        "policy_socket_is_rejected_without_blocking",
        &control,
        "socket",
        "policy_non_regular_socket",
        &invalid,
    );
    assert_invalid_policy(
        "policy_socket_is_rejected_without_blocking",
        "policy_non_regular_socket",
        &invalid,
    );
}

#[test]
fn policy_invalid_utf8_is_rejected() {
    run_transport_case(
        "policy_invalid_utf8_is_rejected",
        "invalid_utf8",
        "policy_invalid_utf8",
        "tests/invalid-utf8-policy.json",
        |repo| {
            write_file(
                repo,
                "tests/invalid-utf8-policy.json",
                [0xff, 0xfe, b'{', b'}'],
            );
        },
    );
}

#[test]
fn policy_malformed_json_is_rejected() {
    run_transport_case(
        "policy_malformed_json_is_rejected",
        "malformed_json",
        "policy_malformed_json",
        "tests/malformed-policy.json",
        |repo| {
            write_file(repo, "tests/malformed-policy.json", b"{\n");
        },
    );
}

#[test]
fn policy_non_object_root_is_rejected() {
    run_transport_case(
        "policy_non_object_root_is_rejected",
        "non_object_root",
        "policy_schema_type",
        "tests/non-object-policy.json",
        |repo| {
            write_file(repo, "tests/non-object-policy.json", b"[]\n");
        },
    );
}

#[test]
fn policy_duplicate_top_level_key_is_rejected() {
    run_transport_case(
        "policy_duplicate_top_level_key_is_rejected",
        "duplicate_top_level_key",
        "policy_duplicate_field",
        "tests/duplicate-top-policy.json",
        |repo| {
            write_file(
                repo,
                "tests/duplicate-top-policy.json",
                b"{\"schema\":1,\"schema\":1}\n",
            );
        },
    );
}

#[test]
fn policy_duplicate_nested_key_is_rejected() {
    run_transport_case(
        "policy_duplicate_nested_key_is_rejected",
        "duplicate_nested_key",
        "policy_duplicate_field",
        "tests/duplicate-nested-policy.json",
        |repo| {
            write_file(
                repo,
                "tests/duplicate-nested-policy.json",
                b"{\"schema\":1,\"nested\":{\"field\":1,\"field\":2}}\n",
            );
        },
    );
}

#[test]
fn policy_oversize_is_rejected() {
    run_transport_case(
        "policy_oversize_is_rejected",
        "oversize",
        "policy_too_large",
        "tests/oversize-policy.json",
        |repo| {
            write_repeated(
                repo,
                "tests/oversize-policy.json",
                "{\"schema\":1,\"padding\":\"",
                &"x".repeat(2 * 1024 * 1024),
                "\"}\n",
            );
        },
    );
}

#[test]
fn policy_excessive_depth_is_rejected() {
    run_transport_case(
        "policy_excessive_depth_is_rejected",
        "excessive_depth",
        "policy_too_deep",
        "tests/deep-policy.json",
        |repo| {
            write_file(repo, "tests/deep-policy.json", deeply_nested_policy(1024));
        },
    );
}

#[test]
fn relative_policy_path_is_root_relative() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let control = run_checker(repo, "tests/test-custody-policy.json", &["--check-markers"]);
    assert_valid_control("relative_policy_path_is_root_relative", &control);

    write_file(repo, "tests/test-custody-policy.json", b"{\n");

    let caller = TempDir::new().expect("alternate caller directory must be created");
    write_file(
        caller.path(),
        "tests/test-custody-policy.json",
        fs::read(source_policy_path()).expect("source policy must be readable"),
    );
    let invalid = run_checker_from(
        caller.path(),
        &repo.join("scripts/check-test-custody.py"),
        repo,
        "tests/test-custody-policy.json",
    );
    print_witness(
        "relative_policy_path_is_root_relative",
        &control,
        "root_relative_policy_path",
        "policy_malformed_json",
        &invalid,
    );
    assert_invalid_policy(
        "relative_policy_path_is_root_relative",
        "policy_malformed_json",
        &invalid,
    );
}

#[test]
fn invalid_policy_blocks_generation_without_replacing_destination() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let control = run_checker(repo, "tests/test-custody-policy.json", &["--check-markers"]);
    assert_valid_control(
        "invalid_policy_blocks_generation_without_replacing_destination",
        &control,
    );

    write_file(repo, "tests/malformed-generation-policy.json", b"{\n");
    let destination = repo.join("tests/generated-baseline.json");
    let sentinel = b"sentinel bytes remain owned by caller\n";
    fs::write(&destination, sentinel).expect("sentinel destination must be written");
    let invalid = run_checker(
        repo,
        "tests/malformed-generation-policy.json",
        &["--generate-baseline", "tests/generated-baseline.json"],
    );
    print_witness(
        "invalid_policy_blocks_generation_without_replacing_destination",
        &control,
        "malformed_json_generation_destination",
        "policy_malformed_json",
        &invalid,
    );
    assert_invalid_policy(
        "invalid_policy_blocks_generation_without_replacing_destination",
        "policy_malformed_json",
        &invalid,
    );
    assert_eq!(
        fs::read(&destination).expect("sentinel destination must remain readable"),
        sentinel,
        "invalid policy must not replace the generation destination"
    );
}
