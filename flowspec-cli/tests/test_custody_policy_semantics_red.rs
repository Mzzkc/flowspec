#![allow(non_snake_case)]

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CHECKER_TIMEOUT: Duration = Duration::from_secs(5);
const TOOL_TIMEOUT: Duration = Duration::from_secs(10);
const SELF_TEST_BANNER: &str = "SELF_TEST: lexical fail-closed marker detection active\n";
const POLICY_PATH: &str = "tests/test-custody-policy.json";
const POLICY_SHA256: &str = "c14829a4d7428166cbd9b6a74cff24fb884d52733b549459f0a8ea3c61acc0d1";
const BASELINE_PATH: &str = "tests/test-custody-baseline.json";
const LEGACY_BASELINE_SHA256: &str =
    "c579c2a68b65be17b2ae3356b3936fe98cdd974016d5f99dfc8003b04391abfe";
const CHECKER_PATH: &str = "scripts/check-test-custody.py";
const HARNESS_PATHS: &[&str] = &[
    "scripts/check-test-custody.py",
    "scripts/flowspec-oracle.py",
    "scripts/validate-oracle-artifacts.py",
];
const WHOLE_FILE_ROOTS: &[(&str, &str)] = &[
    ("tests/", "tests/policy_semantics_fixture_sentinel.py"),
    (
        "flowspec/tests/",
        "flowspec/tests/policy_semantics_fixture_sentinel.rs",
    ),
    (
        "flowspec-cli/tests/",
        "flowspec-cli/tests/policy_semantics_fixture_sentinel.rs",
    ),
];
const MIXED_RUST_ROOTS: &[(&str, &str)] = &[
    (
        "flowspec/src/",
        "flowspec/src/policy_semantics_fixture_sentinel.rs",
    ),
    (
        "flowspec-cli/src/",
        "flowspec-cli/src/policy_semantics_fixture_sentinel.rs",
    ),
];
const EXPECTED_TRACKED: &[&str] = &[
    "flowspec-cli/src/policy_semantics_fixture_sentinel.rs",
    "flowspec-cli/tests/policy_semantics_fixture_sentinel.rs",
    "flowspec/src/policy_semantics_fixture_sentinel.rs",
    "flowspec/tests/policy_semantics_fixture_sentinel.rs",
    "scripts/check-test-custody.py",
    "scripts/flowspec-oracle.py",
    "scripts/validate-oracle-artifacts.py",
    "tests/policy_semantics_fixture_sentinel.py",
    "tests/test-custody-baseline.json",
    "tests/test-custody-policy.json",
];

#[derive(Debug)]
struct MutationRecord {
    before: Value,
    operation: &'static str,
    after: Value,
    semantic_pointer: &'static str,
    resulting_policy_sha256: String,
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn source_path(relative: &str) -> PathBuf {
    repo_root().join(relative)
}

fn finish_bounded(mut child: std::process::Child, timeout: Duration, context: &str) -> Output {
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .unwrap_or_else(|error| {
                panic!("SEMANTICS_PROCESS_FAILURE {context}: poll failed: {error}")
            })
            .is_some()
        {
            return child.wait_with_output().unwrap_or_else(|error| {
                panic!("SEMANTICS_PROCESS_FAILURE {context}: collect failed: {error}")
            });
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!(
                "SEMANTICS_PROCESS_FAILURE {context}: subprocess exceeded {} ms and was killed and reaped",
                timeout.as_millis()
            );
        }
        thread::sleep(Duration::from_millis(20));
    }
}

fn run_bounded(mut command: Command, timeout: Duration, context: &str) -> Output {
    let child = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| {
            panic!("SEMANTICS_PROCESS_FAILURE {context}: spawn failed: {error}")
        });
    finish_bounded(child, timeout, context)
}

fn run_bounded_with_input(
    mut command: Command,
    input: &[u8],
    timeout: Duration,
    context: &str,
) -> Output {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| {
            panic!("SEMANTICS_PROCESS_FAILURE {context}: spawn failed: {error}")
        });
    child
        .stdin
        .take()
        .expect("SEMANTICS_PROCESS_FAILURE piped stdin must exist")
        .write_all(input)
        .unwrap_or_else(|error| {
            panic!("SEMANTICS_PROCESS_FAILURE {context}: stdin write failed: {error}")
        });
    finish_bounded(child, timeout, context)
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

fn git(repo: &Path, args: &[&str]) -> Output {
    let mut command = Command::new("git");
    command.current_dir(repo).args(args);
    let output = run_bounded(command, TOOL_TIMEOUT, &format!("git {args:?}"));
    assert!(
        output.status.success(),
        "SEMANTICS_FIXTURE_FAILURE git {args:?}: stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
    output
}

fn git_text(repo: &Path, args: &[&str]) -> String {
    String::from_utf8(git(repo, args).stdout)
        .expect("SEMANTICS_FIXTURE_FAILURE git output must be UTF-8")
        .trim()
        .to_owned()
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let command = Command::new("sha256sum");
    let output = run_bounded_with_input(command, bytes, TOOL_TIMEOUT, "sha256sum stdin");
    assert!(
        output.status.success(),
        "SEMANTICS_PROCESS_FAILURE sha256sum stdin: stdout={} stderr={}",
        stdout(&output),
        stderr(&output)
    );
    String::from_utf8(output.stdout)
        .expect("SEMANTICS_PROCESS_FAILURE sha256sum output must be UTF-8")
        .split_whitespace()
        .next()
        .expect("SEMANTICS_PROCESS_FAILURE sha256sum must print a digest")
        .to_owned()
}

fn sha256_file(path: &Path) -> String {
    sha256_bytes(&fs::read(path).unwrap_or_else(|error| {
        panic!(
            "SEMANTICS_FIXTURE_FAILURE cannot read {} for digest: {error}",
            path.display()
        )
    }))
}

fn write_file(repo: &Path, relative: &str, bytes: impl AsRef<[u8]>) {
    let path = repo.join(relative);
    fs::create_dir_all(
        path.parent()
            .expect("SEMANTICS_FIXTURE_FAILURE parent required"),
    )
    .unwrap_or_else(|error| {
        panic!("SEMANTICS_FIXTURE_FAILURE cannot create parent for {relative}: {error}")
    });
    fs::write(&path, bytes).unwrap_or_else(|error| {
        panic!("SEMANTICS_FIXTURE_FAILURE cannot write {relative}: {error}")
    });
}

fn copy_authority(repo: &Path, relative: &str) {
    let destination = repo.join(relative);
    fs::create_dir_all(
        destination
            .parent()
            .expect("SEMANTICS_FIXTURE_FAILURE authority parent required"),
    )
    .unwrap_or_else(|error| {
        panic!("SEMANTICS_FIXTURE_FAILURE cannot create parent for {relative}: {error}")
    });
    fs::copy(source_path(relative), &destination).unwrap_or_else(|error| {
        panic!("SEMANTICS_FIXTURE_FAILURE cannot copy authority {relative}: {error}")
    });
}

fn assert_regular(repo: &Path, relative: &str) {
    let metadata = fs::symlink_metadata(repo.join(relative)).unwrap_or_else(|error| {
        panic!("SEMANTICS_FIXTURE_FAILURE missing inventory path {relative}: {error}")
    });
    assert!(
        metadata.file_type().is_file(),
        "SEMANTICS_FIXTURE_FAILURE {relative} must be a regular non-symlink file"
    );
}

fn policy_json(repo: &Path) -> Value {
    serde_json::from_slice(
        &fs::read(repo.join(POLICY_PATH))
            .expect("SEMANTICS_FIXTURE_FAILURE policy must be readable"),
    )
    .expect("SEMANTICS_FIXTURE_FAILURE policy must parse as JSON")
}

fn assert_policy_inventory(repo: &Path) {
    let policy = policy_json(repo);
    assert_eq!(
        policy.pointer("/whole_file_roots"),
        Some(&json!(["tests/", "flowspec/tests/", "flowspec-cli/tests/"])),
        "SEMANTICS_FIXTURE_FAILURE canonical whole-file roots changed"
    );
    assert_eq!(
        policy.pointer("/whole_file_harness_paths"),
        Some(&json!([
            "scripts/check-test-custody.py",
            "scripts/flowspec-oracle.py",
            "scripts/validate-oracle-artifacts.py"
        ])),
        "SEMANTICS_FIXTURE_FAILURE canonical harness paths changed"
    );
    assert_eq!(
        policy.pointer("/mixed_rust_roots"),
        Some(&json!(["flowspec/src/", "flowspec-cli/src/"])),
        "SEMANTICS_FIXTURE_FAILURE canonical mixed-Rust roots changed"
    );
    assert_eq!(
        policy.pointer("/control_baseline/path"),
        Some(&json!(BASELINE_PATH)),
        "SEMANTICS_FIXTURE_FAILURE canonical baseline path changed"
    );
}

// BEGIN BASELINE SOURCE-BINDING HELPER
fn baseline_blob(repo: &Path) -> Vec<u8> {
    let entry = git(repo, &["ls-tree", "HEAD", "--", BASELINE_PATH]);
    let listing = String::from_utf8(entry.stdout)
        .expect("BASELINE_BINDING_FAILURE Git tree entry must be UTF-8");
    let (metadata, path) = listing
        .trim_end_matches('\n')
        .split_once('\t')
        .expect("BASELINE_BINDING_FAILURE baseline Git tree entry missing");
    let fields = metadata.split_whitespace().collect::<Vec<_>>();
    assert!(
        path == BASELINE_PATH
            && fields.len() == 3
            && matches!(fields[0], "100644" | "100755")
            && fields[1] == "blob",
        "BASELINE_BINDING_FAILURE baseline must be a regular committed Git blob"
    );
    git(repo, &["show", &format!("HEAD:{BASELINE_PATH}")]).stdout
}

fn regular_baseline_bytes(repo: &Path) -> Vec<u8> {
    let path = repo.join(BASELINE_PATH);
    let metadata = fs::symlink_metadata(&path).unwrap_or_else(|error| {
        panic!(
            "BASELINE_BINDING_FAILURE missing baseline {}: {error}",
            path.display()
        )
    });
    assert!(
        metadata.file_type().is_file(),
        "BASELINE_BINDING_FAILURE baseline must be a regular non-symlink file: {}",
        path.display()
    );
    fs::read(&path).unwrap_or_else(|error| {
        panic!(
            "BASELINE_BINDING_FAILURE unreadable baseline {}: {error}",
            path.display()
        )
    })
}

fn assert_baseline_bytes_bound_from(source: &Path, scratch: &Path) {
    assert_eq!(
        git_text(scratch, &["status", "--porcelain"]),
        "",
        "BASELINE_BINDING_FAILURE scratch repository must be clean"
    );
    let source_object = baseline_blob(source);
    let source_worktree = regular_baseline_bytes(source);
    let scratch_object = baseline_blob(scratch);
    let scratch_worktree = regular_baseline_bytes(scratch);
    let digests = [
        sha256_bytes(&source_object),
        sha256_bytes(&source_worktree),
        sha256_bytes(&scratch_object),
        sha256_bytes(&scratch_worktree),
    ];
    assert!(
        source_object == source_worktree
            && source_object == scratch_object
            && source_object == scratch_worktree,
        "BASELINE_BINDING_FAILURE baseline bytes differ across source object/worktree and scratch object/worktree: {digests:?}"
    );

    let parsed: Value = serde_json::from_slice(&source_object)
        .expect("BASELINE_BINDING_FAILURE baseline must be valid JSON");
    let object = parsed
        .as_object()
        .expect("BASELINE_BINDING_FAILURE baseline must be a JSON object");
    if object.len() == 3
        && !object.contains_key("schema_version")
        && object.contains_key("files")
        && object.contains_key("source_sha")
        && object.contains_key("tracked_test_paths")
    {
        assert_eq!(
            digests[0], LEGACY_BASELINE_SHA256,
            "BASELINE_BINDING_FAILURE legacy v1 baseline Git object digest changed"
        );
        assert!(
            object["files"].is_object()
                && object["source_sha"].is_string()
                && object["tracked_test_paths"].is_array(),
            "BASELINE_BINDING_FAILURE malformed legacy v1 baseline"
        );
    } else if object.get("schema_version") == Some(&json!(2)) {
        assert!(
            object
                .get("source_sha")
                .and_then(Value::as_str)
                .is_some_and(|value| !value.is_empty()),
            "BASELINE_BINDING_FAILURE v2 source_sha missing"
        );
        for key in ["schema", "policy_path", "policy_sha256"] {
            assert!(
                object.get(key).is_some_and(Value::is_string),
                "BASELINE_BINDING_FAILURE v2 required string key missing: {key}"
            );
        }
        for key in ["whole_files", "rust_regions"] {
            assert!(
                object.get(key).is_some_and(Value::is_object),
                "BASELINE_BINDING_FAILURE v2 required object key missing: {key}"
            );
        }
        for key in ["executable_identities", "rustdoc_identities"] {
            assert!(
                object.get(key).is_some_and(Value::is_array),
                "BASELINE_BINDING_FAILURE v2 required array key missing: {key}"
            );
        }
    } else {
        panic!("BASELINE_BINDING_FAILURE unsupported baseline shape or schema version");
    }
}

fn assert_baseline_bytes_bound(repo: &Path) {
    assert_baseline_bytes_bound_from(&repo_root(), repo);
}
// END BASELINE SOURCE-BINDING HELPER
fn assert_clean_fixture(repo: &Path) {
    assert_eq!(
        git_text(repo, &["status", "--porcelain"]),
        "",
        "SEMANTICS_PRECONDITION_FAILURE scratch Git state must be clean"
    );

    let tracked = git_text(repo, &["ls-files"])
        .lines()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let expected = EXPECTED_TRACKED
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        tracked, expected,
        "SEMANTICS_FIXTURE_FAILURE tracked canonical inventory must be exact"
    );

    for relative in EXPECTED_TRACKED {
        assert_regular(repo, relative);
    }
    for (root, sentinel) in WHOLE_FILE_ROOTS.iter().chain(MIXED_RUST_ROOTS) {
        assert!(
            repo.join(root).is_dir(),
            "SEMANTICS_FIXTURE_FAILURE canonical root {root} must be a directory"
        );
        assert_regular(repo, sentinel);
    }

    assert_eq!(
        sha256_file(&repo.join(POLICY_PATH)),
        POLICY_SHA256,
        "SEMANTICS_FIXTURE_FAILURE scratch policy must equal canonical authority"
    );
    assert_baseline_bytes_bound(repo);
    for relative in HARNESS_PATHS {
        assert_eq!(
            sha256_file(&repo.join(relative)),
            sha256_file(&source_path(relative)),
            "SEMANTICS_FIXTURE_FAILURE harness {relative} must be byte-identical to its distinct source authority"
        );
    }
    assert_eq!(
        sha256_file(&repo.join(CHECKER_PATH)),
        sha256_file(&source_path(CHECKER_PATH)),
        "SEMANTICS_FIXTURE_FAILURE candidate checker bytes changed in scratch"
    );
    assert_policy_inventory(repo);
}

fn init_clean_repo() -> TempDir {
    let temp = TempDir::new().expect("SEMANTICS_SETUP_FAILURE scratch repository creation failed");
    let repo = temp.path();
    git(repo, &["init"]);
    git(
        repo,
        &["config", "user.email", "custody-semantics@example.invalid"],
    );
    git(repo, &["config", "user.name", "Custody Semantics RED"]);

    copy_authority(repo, POLICY_PATH);
    copy_authority(repo, BASELINE_PATH);
    for relative in HARNESS_PATHS {
        copy_authority(repo, relative);
    }

    write_file(
        repo,
        "tests/policy_semantics_fixture_sentinel.py",
        b"SEMANTICS_FIXTURE_SENTINEL = True\n",
    );
    for (_, sentinel) in WHOLE_FILE_ROOTS.iter().skip(1).chain(MIXED_RUST_ROOTS) {
        write_file(
            repo,
            sentinel,
            b"const POLICY_SEMANTICS_FIXTURE_SENTINEL: () = ();\n",
        );
    }

    git(repo, &["add", "--all"]);
    git(
        repo,
        &["commit", "-m", "clean complete custody semantics fixture"],
    );
    assert_clean_fixture(repo);
    temp
}

fn run_self_test(repo: &Path) -> Output {
    let mut command = Command::new("python3");
    command
        .current_dir(repo)
        .arg(CHECKER_PATH)
        .arg("--root")
        .arg(".")
        .arg("--policy")
        .arg(POLICY_PATH)
        .arg("--self-test");
    run_bounded(command, CHECKER_TIMEOUT, "custody checker --self-test")
}

fn assert_valid_control(identity: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        0,
        "SEMANTICS_PRECONDITION_FAILURE {identity}: valid --self-test control must exit 0; stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stdout(output),
        "",
        "SEMANTICS_PRECONDITION_FAILURE {identity}: valid control stdout must be empty"
    );
    assert_eq!(
        stderr(output),
        SELF_TEST_BANNER,
        "SEMANTICS_PRECONDITION_FAILURE {identity}: valid control stderr must be exact"
    );
}

fn pointer_tokens(pointer: &str) -> Vec<String> {
    assert!(
        pointer.starts_with('/'),
        "SEMANTICS_MUTATION_FAILURE mutation pointer must be absolute: {pointer}"
    );
    pointer
        .split('/')
        .skip(1)
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn replace_subtree(document: &mut Value, pointer: &str, replacement: Value) {
    let mut tokens = pointer_tokens(pointer);
    let leaf = tokens
        .pop()
        .expect("SEMANTICS_MUTATION_FAILURE negative pointer must own a subtree");
    let mut parent = document;
    for token in tokens {
        parent = match parent {
            Value::Object(map) => map.get_mut(&token).unwrap_or_else(|| {
                panic!("SEMANTICS_MUTATION_FAILURE missing object token {token} in {pointer}")
            }),
            Value::Array(items) => {
                let index = token.parse::<usize>().unwrap_or_else(|_| {
                    panic!("SEMANTICS_MUTATION_FAILURE invalid array token {token} in {pointer}")
                });
                items.get_mut(index).unwrap_or_else(|| {
                    panic!("SEMANTICS_MUTATION_FAILURE missing array item {index} in {pointer}")
                })
            }
            other => {
                panic!("SEMANTICS_MUTATION_FAILURE non-container parent for {pointer}: {other:?}")
            }
        };
    }
    match parent {
        Value::Object(map) => {
            let target = map.get_mut(&leaf).unwrap_or_else(|| {
                panic!("SEMANTICS_MUTATION_FAILURE missing object leaf {leaf} in {pointer}")
            });
            *target = replacement;
        }
        Value::Array(items) => {
            let index = leaf.parse::<usize>().unwrap_or_else(|_| {
                panic!("SEMANTICS_MUTATION_FAILURE invalid array leaf {leaf} in {pointer}")
            });
            *items.get_mut(index).unwrap_or_else(|| {
                panic!("SEMANTICS_MUTATION_FAILURE missing array leaf {index} in {pointer}")
            }) = replacement;
        }
        other => {
            panic!("SEMANTICS_MUTATION_FAILURE non-container leaf parent for {pointer}: {other:?}")
        }
    }
}

fn escape_token(token: &str) -> String {
    token.replace('~', "~0").replace('/', "~1")
}

fn join_pointer(parent: &str, token: &str) -> String {
    if parent.is_empty() {
        format!("/{}", escape_token(token))
    } else {
        format!("{}/{}", parent, escape_token(token))
    }
}

fn collect_diffs(before: &Value, after: &Value, pointer: &str, diffs: &mut Vec<String>) {
    if before == after {
        return;
    }
    match (before, after) {
        (Value::Object(left), Value::Object(right)) => {
            let keys = left
                .keys()
                .chain(right.keys())
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            for key in keys {
                let child = join_pointer(pointer, key);
                match (left.get(key), right.get(key)) {
                    (Some(left), Some(right)) => collect_diffs(left, right, &child, diffs),
                    _ => diffs.push(child),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                let child = join_pointer(pointer, &index.to_string());
                match (left.get(index), right.get(index)) {
                    (Some(left), Some(right)) => collect_diffs(left, right, &child, diffs),
                    _ => diffs.push(child),
                }
            }
        }
        _ => diffs.push(pointer.to_owned()),
    }
}

fn compact_json(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("SEMANTICS_MUTATION_FAILURE compact JSON serialization failed")
}

fn pretty_policy_bytes(value: &Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .expect("SEMANTICS_MUTATION_FAILURE policy JSON serialization failed");
    bytes.push(b'\n');
    bytes
}

fn same_json_kind(left: &Value, right: &Value) -> bool {
    matches!(
        (left, right),
        (Value::Null, Value::Null)
            | (Value::Bool(_), Value::Bool(_))
            | (Value::Number(_), Value::Number(_))
            | (Value::String(_), Value::String(_))
            | (Value::Array(_), Value::Array(_))
            | (Value::Object(_), Value::Object(_))
    )
}

fn is_structural_array_class(mutation_class: &str) -> bool {
    matches!(
        mutation_class,
        "shorter" | "longer" | "duplicate" | "reorder"
    )
}

fn apply_and_prove_mutation(
    identity: &str,
    repo: &Path,
    pointer: &'static str,
    mutation_class: &'static str,
    value_json: &str,
    expected_value_sha256: &str,
) -> MutationRecord {
    assert_eq!(
        git_text(repo, &["status", "--porcelain"]),
        "",
        "SEMANTICS_PRECONDITION_FAILURE {identity}: fixture must remain clean before mutation"
    );
    let canonical = policy_json(repo);
    let before = canonical
        .pointer(pointer)
        .unwrap_or_else(|| {
            panic!("SEMANTICS_MUTATION_FAILURE {identity}: pointer does not resolve: {pointer}")
        })
        .clone();
    let after_value: Value = serde_json::from_str(value_json).unwrap_or_else(|error| {
        panic!("SEMANTICS_MUTATION_FAILURE {identity}: embedded mutation JSON invalid: {error}")
    });
    assert!(
        same_json_kind(&before, &after_value),
        "SEMANTICS_MUTATION_FAILURE {identity}: mutation changes structural JSON kind"
    );
    assert_ne!(
        before, after_value,
        "SEMANTICS_MUTATION_FAILURE {identity}: mutation equals canonical value"
    );
    assert_eq!(
        sha256_bytes(&compact_json(&after_value)),
        expected_value_sha256,
        "SEMANTICS_MUTATION_FAILURE {identity}: compact UTF-8 mutation-value digest changed"
    );

    let mut reconstructed = canonical.clone();
    replace_subtree(&mut reconstructed, pointer, after_value.clone());
    assert_eq!(
        reconstructed.pointer(pointer),
        Some(&after_value),
        "SEMANTICS_MUTATION_FAILURE {identity}: replacement subtree differs from ledger value"
    );

    if is_structural_array_class(mutation_class) {
        assert!(
            before.is_array() && after_value.is_array(),
            "SEMANTICS_MUTATION_FAILURE {identity}: structural array case must replace owning arrays"
        );
        assert!(
            !pointer.rsplit('/').next().unwrap_or_default().chars().all(|c| c.is_ascii_digit()),
            "SEMANTICS_MUTATION_FAILURE {identity}: structural array owner cannot be an item pointer"
        );
    } else {
        let mut diffs = Vec::new();
        collect_diffs(&canonical, &reconstructed, "", &mut diffs);
        assert_eq!(
            diffs,
            vec![pointer.to_owned()],
            "SEMANTICS_MUTATION_FAILURE {identity}: scalar/item mutation must change exactly its descendant pointer"
        );
    }

    let expected_bytes = pretty_policy_bytes(&reconstructed);
    let record = MutationRecord {
        before: before.clone(),
        operation: mutation_class,
        after: after_value.clone(),
        semantic_pointer: pointer,
        resulting_policy_sha256: sha256_bytes(&expected_bytes),
    };
    fs::write(repo.join(POLICY_PATH), &expected_bytes).unwrap_or_else(|error| {
        panic!("SEMANTICS_MUTATION_FAILURE {identity}: cannot write reconstructed policy: {error}")
    });
    let executed_bytes = fs::read(repo.join(POLICY_PATH)).unwrap_or_else(|error| {
        panic!("SEMANTICS_MUTATION_FAILURE {identity}: cannot reread executed policy: {error}")
    });
    assert_eq!(
        executed_bytes, expected_bytes,
        "SEMANTICS_MUTATION_FAILURE {identity}: checker bytes must equal full reconstruction"
    );
    assert_eq!(
        sha256_bytes(&executed_bytes),
        record.resulting_policy_sha256,
        "SEMANTICS_MUTATION_FAILURE {identity}: resulting full-policy digest changed"
    );

    let mut replay = canonical;
    replace_subtree(&mut replay, record.semantic_pointer, record.after.clone());
    assert_eq!(
        pretty_policy_bytes(&replay),
        executed_bytes,
        "SEMANTICS_MUTATION_FAILURE {identity}: recorded operation replay must reproduce executed bytes"
    );
    assert_eq!(record.before, before);
    assert_eq!(record.operation, mutation_class);
    assert_eq!(
        git_text(repo, &["status", "--porcelain"]),
        format!("M {POLICY_PATH}"),
        "SEMANTICS_MUTATION_FAILURE {identity}: exactly the policy file must be modified"
    );
    record
}

fn assert_future_semantic_rejection(pointer: &str, reason: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        1,
        "future semantic rejection must exit 1 with exact reason and pointer"
    );
    assert_eq!(
        stdout(output),
        "",
        "future semantic rejection must exit 1 with exact reason and pointer"
    );
    assert_eq!(
        stderr(output),
        format!("ERROR: {reason}: {pointer}\n"),
        "future semantic rejection must exit 1 with exact reason and pointer"
    );
}

fn run_semantics_case(
    identity: &str,
    pointer: &'static str,
    mutation_class: &'static str,
    reason: &'static str,
    value_json: &str,
    value_sha256: &str,
) {
    let temp = init_clean_repo();
    let repo = temp.path();
    let control = run_self_test(repo);
    assert_valid_control(identity, &control);

    let record = apply_and_prove_mutation(
        identity,
        repo,
        pointer,
        mutation_class,
        value_json,
        value_sha256,
    );
    assert_eq!(
        record.semantic_pointer, pointer,
        "SEMANTICS_MUTATION_FAILURE {identity}: semantic owner changed"
    );

    let observed = run_self_test(repo);
    println!(
        "POLICY_SEMANTICS_RED case_id={identity} pointer={pointer} mutation_class={mutation_class} expected_reason={reason} value_sha256={value_sha256} semantic_owner={pointer} control_exit={} observed_exit={} failure_site=future_semantic_assertion",
        exit_code(&control),
        exit_code(&observed)
    );
    assert_future_semantic_rejection(pointer, reason, &observed);
}

#[test]
fn canonical_policy_v1() {
    let temp = init_clean_repo();
    let repo = temp.path();
    assert_eq!(
        sha256_file(&repo.join(POLICY_PATH)),
        POLICY_SHA256,
        "SEMANTICS_PRECONDITION_FAILURE canonical policy digest changed"
    );
    let control = run_self_test(repo);
    assert_valid_control("canonical_policy_v1", &control);
}

macro_rules! semantics_case {
    ($name:ident, $pointer:expr, $mutation_class:expr, $reason:expr, $value:expr, $value_sha:expr) => {
        #[test]
        fn $name() {
            run_semantics_case(
                stringify!($name),
                $pointer,
                $mutation_class,
                $reason,
                $value,
                $value_sha,
            );
        }
    };
}

semantics_case!(
    boolean_marker_tracked_regular_utf8_source_false,
    "/marker_scan/tracked_regular_utf8_source",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_marker_path_exclusions_allowed_true,
    "/marker_scan/path_exclusions_allowed",
    "boolean_flip",
    "policy_unsafe_value",
    r#"true"#,
    "b5bea41b6c623f7c09f1bf24dcae58ebab3c0cdd90ad966bc43a45b44867e12b"
);
semantics_case!(
    boolean_rust_cfg_test_predicate_false,
    "/rust_region_rules/cfg_test_predicate",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_rust_external_modules_false,
    "/rust_region_rules/external_modules_under_test_cfg",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_rust_executable_rustdoc_fences_false,
    "/rust_region_rules/executable_rustdoc_fences",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_rust_local_macro_invocations_false,
    "/rust_region_rules/local_macro_invocations_inside_custodied_regions",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_rust_fail_on_unclassified_false,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_include_str_resolution_false,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_include_bytes_resolution_false,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_include_regular_files_only_false,
    "/rust_include_edges/repository_contained_regular_files_only",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_unresolved_include_is_error_false,
    "/rust_include_edges/unresolved_include_is_error",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_baseline_self_exclusion_false,
    "/control_baseline/excluded_from_own_file_hash_map",
    "boolean_flip",
    "policy_invalid_binding",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_cargo_workspace_false,
    "/cargo_inventory/platform_contract/workspace",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_cargo_all_features_false,
    "/cargo_inventory/platform_contract/all_features",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_cargo_exact_identity_set_false,
    "/cargo_inventory/exact_identity_set_required",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    boolean_cargo_target_qualified_false,
    "/cargo_inventory/target_qualified_identities",
    "boolean_flip",
    "policy_unsafe_value",
    r#"false"#,
    "fcbcf165908dd18a9e49f7ff27810176db8e9f63b4352213741664245224f8aa"
);
semantics_case!(
    scalar_policy_version_future,
    "/policy_version",
    "exact_value_substitution",
    "policy_unsupported_version",
    r#""phase-01-custody-closeout-v2""#,
    "033f1b0ecb01887eac2b0720886f017e50858a6042fe5772365949a0bbbbc67a"
);
semantics_case!(
    scalar_baseline_path_alternative,
    "/control_baseline/path",
    "exact_value_substitution",
    "policy_invalid_binding",
    r#""tests/alternate-custody-baseline.json""#,
    "2525c7d4df78883fe0ec97be530d921b0cd8bc0bb633dd93f9f9c84ca1b2dc4b"
);
semantics_case!(
    scalar_baseline_reason_alternative,
    "/control_baseline/self_exclusion_reason",
    "exact_value_substitution",
    "policy_invalid_binding",
    r#""baseline excludes its own hash to avoid recursion""#,
    "01f0c39aa726757962a17c0518c564ddd500f557aa167fb0df3b66fdf461377e"
);
semantics_case!(
    scalar_platform_os_alternative,
    "/cargo_inventory/platform_contract/os",
    "exact_value_substitution",
    "policy_invalid_binding",
    r#""windows""#,
    "e2946cabbb26da3c076d98f3395e9fe1e0725ba39df2fe6e79f955408c07ed90"
);
semantics_case!(
    version_empty,
    "/policy_version",
    "empty",
    "policy_unsupported_version",
    r#""""#,
    "12ae32cb1ec02d01eda3581b127c1fee3b0dc53572ed6baf239721a03d82e126"
);
semantics_case!(
    version_ascii_whitespace_only,
    "/policy_version",
    "whitespace_only",
    "policy_unsupported_version",
    r#""   ""#,
    "7c1e8804d74233306ba928123341fec515e7bd30473255c76e37d6cfe3d8105c"
);
semantics_case!(
    version_leading_ascii_whitespace,
    "/policy_version",
    "leading_whitespace",
    "policy_unsupported_version",
    r#"" phase-01-custody-closeout-red""#,
    "7726ac4e4b46bd3213eeae64d2795ed1bb95d069055f54c310d6487f72c6df50"
);
semantics_case!(
    version_trailing_ascii_whitespace,
    "/policy_version",
    "trailing_whitespace",
    "policy_unsupported_version",
    r#""phase-01-custody-closeout-red ""#,
    "965fab6ae511bdf24e8ae7a12070b191b31c24e8dd5efad8e77a38f12fe1a2bf"
);
semantics_case!(
    version_case_variant,
    "/policy_version",
    "case_variant",
    "policy_unsupported_version",
    r#""Phase-01-custody-closeout-red""#,
    "fb9593c95a0afb75eae57876e6c8b0051cf279419239dc1ae6cbd55c6651a7b8"
);
semantics_case!(
    version_prefix_acceptance,
    "/policy_version",
    "prefix",
    "policy_unsupported_version",
    r#""xphase-01-custody-closeout-red""#,
    "0d1e7def0891b3a4d5d9a9fb1e0fb6ba4fbbefce87f0f80d7ce42c9e8885864c"
);
semantics_case!(
    version_suffix_acceptance,
    "/policy_version",
    "suffix",
    "policy_unsupported_version",
    r#""phase-01-custody-closeout-red-extra""#,
    "5ff841974126c3f58f0785a99cc3d7575ddb895a3bb42b4d917ad1da28c3b4b6"
);
semantics_case!(
    version_unicode_hyphen,
    "/policy_version",
    "unicode_confusable",
    "policy_unsupported_version",
    r#""phase\u201001-custody-closeout-red""#,
    "e01b1932c16109712c785affd40771800670726e946c544fd6f988e4fa954ff1"
);
semantics_case!(
    shared_platform_case_fold,
    "/cargo_inventory/platform_contract/os",
    "case_fold",
    "policy_invalid_binding",
    r#""LINUX""#,
    "f653cc539d6071f21d24248dcaa76e18c2897d3a372cc32b55e1d5124ae89b9e"
);
semantics_case!(
    shared_platform_trailing_trim,
    "/cargo_inventory/platform_contract/os",
    "trailing_whitespace",
    "policy_invalid_binding",
    r#""linux ""#,
    "17690f4f5ae5f9f5c6432b2e92d681adeba3e91325f5f133bf2d9774b4843f72"
);
semantics_case!(
    shared_baseline_reason_confusable,
    "/control_baseline/self_exclusion_reason",
    "unicode_confusable",
    "policy_invalid_binding",
    r#""baseline s\u0435lf hash has no fixed point""#,
    "dccec57068339d99fdebb566b2e07bb4e6760035e8087ab37bd39446f3d0048c"
);
semantics_case!(
    path_nul,
    "/whole_file_roots/0",
    "nul",
    "policy_noncanonical_path",
    r#""tests/\u0000/""#,
    "44cf46a0730b7345c58e1568687cf2aae64209cdb2c36b0c0d4913750799e6b6"
);
semantics_case!(
    path_backslash_separator,
    "/whole_file_roots/0",
    "backslash_separator",
    "policy_noncanonical_path",
    r#""tests\\fixtures/""#,
    "b28924aed7eaf688a0bf26ec0408d38ad9301c5a215b3c83ab071a0f66771e5c"
);
semantics_case!(
    path_absolute_posix,
    "/whole_file_roots/0",
    "absolute_posix",
    "policy_noncanonical_path",
    r#""/tests/""#,
    "40546de8ec1c305b6f7b4e99f49110edcea754a0ea442773e3dd0c268c96ccb0"
);
semantics_case!(
    path_windows_drive_prefix,
    "/whole_file_roots/0",
    "windows_drive_prefix",
    "policy_noncanonical_path",
    r#""C:/tests/""#,
    "bbd0a2f652fe25e3c42ea05e86539d411be5c68e6974fa7573b4884ea2cf6075"
);
semantics_case!(
    path_dot_segment,
    "/whole_file_roots/0",
    "dot_segment",
    "policy_noncanonical_path",
    r#""tests/./""#,
    "1cf981f8f7e9adf92909c45764e896a55cc9e7d6295acf44d6f8a519b394c805"
);
semantics_case!(
    path_parent_segment,
    "/whole_file_roots/0",
    "parent_segment",
    "policy_noncanonical_path",
    r#""tests/../""#,
    "11dedc01f43fb946fa72ae8b34f27cde6ceb8920238886379eaef885b05f9dc6"
);
semantics_case!(
    path_repeated_separator,
    "/whole_file_roots/0",
    "repeated_separator",
    "policy_noncanonical_path",
    r#""tests//""#,
    "0c134ebd3116d7cacb4c70f054ab5ed45e6fbe57a5c11b96157abb5d7caaa330"
);
semantics_case!(
    path_root_missing_trailing_slash,
    "/whole_file_roots/0",
    "root_missing_trailing_slash",
    "policy_noncanonical_path",
    r#""tests""#,
    "68f05093bc600659c0aecd265fad80c71a658cc857e91cdfeeee782eb670c200"
);
semantics_case!(
    path_file_has_trailing_slash,
    "/whole_file_harness_paths/0",
    "file_has_trailing_slash",
    "policy_noncanonical_path",
    r#""scripts/check-test-custody.py/""#,
    "9b20ad42289466fe166359715bd25411cda5b328c7ca6e3bd8a6b16a20e86802"
);
semantics_case!(
    path_empty,
    "/control_baseline/path",
    "empty",
    "policy_noncanonical_path",
    r#""""#,
    "12ae32cb1ec02d01eda3581b127c1fee3b0dc53572ed6baf239721a03d82e126"
);
semantics_case!(
    path_ascii_control,
    "/whole_file_harness_paths/1",
    "ascii_control",
    "policy_noncanonical_path",
    r#""scripts/\u001foracle.py""#,
    "74a9ded893dbb49fb684273f122cd3d01fa525ad22af2f56fdd8e9cd5cf12804"
);
semantics_case!(
    path_unicode_separator,
    "/mixed_rust_roots/0",
    "unicode_separator",
    "policy_noncanonical_path",
    r#""flowspec\u2215src/""#,
    "fcaedfcd3a545cd66edf83f04bc342b0f0aa384eb5971f42459dcb390106f60f"
);
semantics_case!(
    path_bidi_control,
    "/whole_file_harness_paths/2",
    "bidi_control",
    "policy_noncanonical_path",
    r#""scripts/\u202evalidate-oracle-artifacts.py""#,
    "28a3d6863d2e29e7cfc0c2cf9075f4412d37c5c518ec0c67a3f97c06e7ffcd95"
);
semantics_case!(
    array_substitute_extension_0,
    "/marker_scan/extensions/0",
    "item_substitution",
    "policy_invalid_entry",
    r#"".c""#,
    "c115df34ae44b67f4af7b530cfe6b8cf2fa468e955268feb1cac6848f65729b1"
);
semantics_case!(
    array_substitute_extension_1,
    "/marker_scan/extensions/1",
    "item_substitution",
    "policy_invalid_entry",
    r#"".mjs""#,
    "8c57421fab1559a6f9e1d6d184c17d004216c34cd9794b0275fef8e22642ab37"
);
semantics_case!(
    array_substitute_extension_2,
    "/marker_scan/extensions/2",
    "item_substitution",
    "policy_invalid_entry",
    r#"".mts""#,
    "f32d42eed56331f2bf67e884ca1b10a3e502ec9750261374ed605d385a47b88b"
);
semantics_case!(
    array_substitute_extension_3,
    "/marker_scan/extensions/3",
    "item_substitution",
    "policy_invalid_entry",
    r#"".cts""#,
    "805d91a138b3a8faeb8ea9f02c1a1f8292c72a0fe77f58387a3be185ae9faa2f"
);
semantics_case!(
    array_substitute_extension_4,
    "/marker_scan/extensions/4",
    "item_substitution",
    "policy_invalid_entry",
    r#"".vue""#,
    "7fbc867637a6b7b085ba545562aa8450b12bfd0066fd29b69b0be6790b40b71a"
);
semantics_case!(
    array_substitute_extension_5,
    "/marker_scan/extensions/5",
    "item_substitution",
    "policy_invalid_entry",
    r#"".pyi""#,
    "74db2353342618de4b06d1faf622784f026c1cc3d31228c580f7f6253d2629c1"
);
semantics_case!(
    array_substitute_root_0,
    "/whole_file_roots/0",
    "item_substitution",
    "policy_invalid_entry",
    r#""spec/""#,
    "3d19d0df2d01ff77681810b4ea51588d273f8931e88ee2820ba86e5045350068"
);
semantics_case!(
    array_substitute_root_1,
    "/whole_file_roots/1",
    "item_substitution",
    "policy_invalid_entry",
    r#""crates/flowspec/tests/""#,
    "97721d46cd95bda62e9a3788233d595ac7c222437cb946d8c305d6aeb82e7c22"
);
semantics_case!(
    array_substitute_root_2,
    "/whole_file_roots/2",
    "item_substitution",
    "policy_invalid_entry",
    r#""crates/flowspec-cli/tests/""#,
    "cd7e43e080be3fbfbb48bafc412538165be712350b4c530c87f3f18d2b6ec831"
);
semantics_case!(
    array_substitute_harness_0,
    "/whole_file_harness_paths/0",
    "item_substitution",
    "policy_invalid_entry",
    r#""scripts/check-custody.py""#,
    "5dee491fbb99b35478c2835cb2ee92cba9c522dda3579cabd3a2635bcf8a75d3"
);
semantics_case!(
    array_substitute_harness_1,
    "/whole_file_harness_paths/1",
    "item_substitution",
    "policy_invalid_entry",
    r#""scripts/alternate-oracle.py""#,
    "0e112384aa75616c4a8c4553ce78478ffa8cceb83cb3d90ccc576400064335ea"
);
semantics_case!(
    array_substitute_harness_2,
    "/whole_file_harness_paths/2",
    "item_substitution",
    "policy_invalid_entry",
    r#""scripts/validate-artifacts.py""#,
    "06209d5fabbcce68ded22ebf24f61f615e4278eccd8fd371a066b1873c71bc43"
);
semantics_case!(
    array_substitute_mixed_root_0,
    "/mixed_rust_roots/0",
    "item_substitution",
    "policy_invalid_entry",
    r#""src/""#,
    "35f6f5f500ad9fba8712df8bf824c6abe31ef68ff4dc2b3f0581c1590e44d6ad"
);
semantics_case!(
    array_substitute_mixed_root_1,
    "/mixed_rust_roots/1",
    "item_substitution",
    "policy_invalid_entry",
    r#""cli/src/""#,
    "7291478abae0bc068f824d6668a472d235a02fa85387abc9520af30228e949df"
);
semantics_case!(
    array_substitute_test_attribute_0,
    "/rust_region_rules/test_attributes/0",
    "item_substitution",
    "policy_invalid_entry",
    r#""tokio::test""#,
    "59d786a3a48fa3baa72794fafdfc030c80671c783f3a0783f21e34b9d4c07602"
);
semantics_case!(
    array_substitute_command_0,
    "/cargo_inventory/enumeration_commands/0",
    "item_substitution",
    "policy_invalid_entry",
    r#""cargo test --workspace --all-features --tests --no-run""#,
    "f6fd5275b3750eb96faa3cab59a1e5626e2186925c022c42669214676161e96c"
);
semantics_case!(
    array_substitute_command_1,
    "/cargo_inventory/enumeration_commands/1",
    "item_substitution",
    "policy_invalid_entry",
    r#""cargo test --workspace --all-features --lib -- --list""#,
    "d340515e6f688bb1a0f24d652ab8eac0e3b9b3f18874d394704d18094987fa17"
);
semantics_case!(
    array_substitute_command_2,
    "/cargo_inventory/enumeration_commands/2",
    "item_substitution",
    "policy_invalid_entry",
    r#""cargo test --workspace --all-features --doc -- --list --format terse""#,
    "bbba8d872d222d11548eb129cbb6922207d44759bb46526ad9845e72bb93ac76"
);
semantics_case!(
    array_short_extensions,
    "/marker_scan/extensions",
    "shorter",
    "policy_invalid_cardinality",
    r#"[".rs",".js",".ts",".tsx",".jsx"]"#,
    "32d05d65d319e81fca11964001b1cca59c0e7a0c873858fa532717a8ac75f51d"
);
semantics_case!(
    array_short_roots,
    "/whole_file_roots",
    "shorter",
    "policy_invalid_cardinality",
    r#"["tests/","flowspec/tests/"]"#,
    "7145afda22d534c36c6e8d78506a01234d1d6c7ce44f0b4200f53fbd2dd9684f"
);
semantics_case!(
    array_short_harnesses,
    "/whole_file_harness_paths",
    "shorter",
    "policy_invalid_cardinality",
    r#"["scripts/check-test-custody.py","scripts/flowspec-oracle.py"]"#,
    "15fc58346cd1875a0848d25785d6f6ac99e83cd381b6ef009bf08868d64ce2c4"
);
semantics_case!(
    array_short_mixed_roots,
    "/mixed_rust_roots",
    "shorter",
    "policy_invalid_cardinality",
    r#"["flowspec/src/"]"#,
    "03e99be4a724b4b187ded6aea28ae84fc31908e12cf46b6e073c366c1e47c398"
);
semantics_case!(
    array_short_test_attributes,
    "/rust_region_rules/test_attributes",
    "shorter",
    "policy_invalid_cardinality",
    r#"[]"#,
    "4f53cda18c2baa0c0354bb5f9a3ecbe5ed12ab4d8e11ba873c2f11161202b945"
);
semantics_case!(
    array_short_commands,
    "/cargo_inventory/enumeration_commands",
    "shorter",
    "policy_invalid_cardinality",
    r#"["cargo test --workspace --all-features --no-run","cargo test --workspace --all-features -- --list"]"#,
    "d050999a1bd9ad63c808a45aa6aec57edcbbc35566043ad8f7e77b98ac0826b9"
);
semantics_case!(
    array_long_extensions,
    "/marker_scan/extensions",
    "longer",
    "policy_invalid_cardinality",
    r#"[".rs",".js",".ts",".tsx",".jsx",".py",".c"]"#,
    "10ee8bebac88e304f6b47fdb5a8517b2218f43d61a86945de2c0e52a34110435"
);
semantics_case!(
    array_long_roots,
    "/whole_file_roots",
    "longer",
    "policy_invalid_cardinality",
    r#"["tests/","flowspec/tests/","flowspec-cli/tests/","examples/tests/"]"#,
    "3fea89d1fed9b301999e389315dbe7292c35a24233ea5629084db6903d9b43fc"
);
semantics_case!(
    array_long_harnesses,
    "/whole_file_harness_paths",
    "longer",
    "policy_invalid_cardinality",
    r#"["scripts/check-test-custody.py","scripts/flowspec-oracle.py","scripts/validate-oracle-artifacts.py","scripts/extra-harness.py"]"#,
    "1732b2ba29b33dd02a75041218e1e8c104d8b5d748fcd19e6f6ddda764243a4a"
);
semantics_case!(
    array_long_mixed_roots,
    "/mixed_rust_roots",
    "longer",
    "policy_invalid_cardinality",
    r#"["flowspec/src/","flowspec-cli/src/","examples/src/"]"#,
    "3bf07c1dffd2a8c4d9daddc800bd1915f52226d3b360a10422fbd5ed0cd91188"
);
semantics_case!(
    array_long_test_attributes,
    "/rust_region_rules/test_attributes",
    "longer",
    "policy_invalid_cardinality",
    r#"["test","tokio::test"]"#,
    "ffcb71292f050d2c9b6503986b67c277349b8e9f9b0fcea38ee43e97fb2f14f5"
);
semantics_case!(
    array_long_policy_test_attributes,
    "/rust_region_rules/policy_test_attributes",
    "longer",
    "policy_invalid_cardinality",
    r#"["test"]"#,
    "ecfd160805b1b0481fd0793c745be3b45d2054582de1c4df5d9b8fa4d78e7fbc"
);
semantics_case!(
    array_long_commands,
    "/cargo_inventory/enumeration_commands",
    "longer",
    "policy_invalid_cardinality",
    r#"["cargo test --workspace --all-features --no-run","cargo test --workspace --all-features -- --list","cargo test --workspace --all-features --doc -- --list","cargo test --workspace --all-features"]"#,
    "91b256c1c15f2119a2e8e03b61d8035b8f80b690dd9e8bbd63883ad521d6e6f2"
);
semantics_case!(
    array_duplicate_extensions,
    "/marker_scan/extensions",
    "duplicate",
    "policy_duplicate_item",
    r#"[".rs",".rs",".ts",".tsx",".jsx",".py"]"#,
    "b0e1b3783c65e084ec081352184fde2ba110843b703cca0fb78b9ed2ec656910"
);
semantics_case!(
    array_duplicate_roots,
    "/whole_file_roots",
    "duplicate",
    "policy_duplicate_item",
    r#"["tests/","tests/","flowspec-cli/tests/"]"#,
    "c62b513e4877a9ee57204d8790d848a7ec2f54afd27cef9e3a4c2ece04dd6d64"
);
semantics_case!(
    array_duplicate_harnesses,
    "/whole_file_harness_paths",
    "duplicate",
    "policy_duplicate_item",
    r#"["scripts/check-test-custody.py","scripts/check-test-custody.py","scripts/validate-oracle-artifacts.py"]"#,
    "57373dda4ce76cfc703bf767141aab2981d9c7186e94f36e16fcdb9becaf9376"
);
semantics_case!(
    array_duplicate_mixed_roots,
    "/mixed_rust_roots",
    "duplicate",
    "policy_duplicate_item",
    r#"["flowspec/src/","flowspec/src/"]"#,
    "69a332c05f5a9f2763e2dc01e5f2ca592a48a7a6697008d6c488a89b03877f7c"
);
semantics_case!(
    array_duplicate_test_attributes,
    "/rust_region_rules/test_attributes",
    "duplicate",
    "policy_duplicate_item",
    r#"["test","test"]"#,
    "2b029cd063446f4082785639a797aac674839c469ce2c1908a95e985264fb871"
);
semantics_case!(
    array_duplicate_commands,
    "/cargo_inventory/enumeration_commands",
    "duplicate",
    "policy_duplicate_item",
    r#"["cargo test --workspace --all-features --no-run","cargo test --workspace --all-features --no-run","cargo test --workspace --all-features --doc -- --list"]"#,
    "078580d01fa89c0e25a7eb0bc6f9130bdebde55c947044e62b55af6fda137b9d"
);
semantics_case!(
    array_reorder_extensions,
    "/marker_scan/extensions",
    "reorder",
    "policy_invalid_order",
    r#"[".js",".rs",".ts",".tsx",".jsx",".py"]"#,
    "527cdf360df4719adc40fdfa00a98affabac4cb8f9112332c61f9827307fa11c"
);
semantics_case!(
    array_reorder_roots,
    "/whole_file_roots",
    "reorder",
    "policy_invalid_order",
    r#"["flowspec/tests/","tests/","flowspec-cli/tests/"]"#,
    "a65fcad470b78242cfba68dd24e447f92d8cdf8c877cbc482db4c7a15bdbcd49"
);
semantics_case!(
    array_reorder_harnesses,
    "/whole_file_harness_paths",
    "reorder",
    "policy_invalid_order",
    r#"["scripts/flowspec-oracle.py","scripts/check-test-custody.py","scripts/validate-oracle-artifacts.py"]"#,
    "95cbe7d8a54945596f455810030ccb9377ee4f213da7d228f85835cd5d64c848"
);
semantics_case!(
    array_reorder_mixed_roots,
    "/mixed_rust_roots",
    "reorder",
    "policy_invalid_order",
    r#"["flowspec-cli/src/","flowspec/src/"]"#,
    "94a3c8eac130604dc4d06969ecb20238462eb90a42dec19fefbe14f1422ae9e6"
);
semantics_case!(
    array_reorder_commands,
    "/cargo_inventory/enumeration_commands",
    "reorder",
    "policy_invalid_order",
    r#"["cargo test --workspace --all-features -- --list","cargo test --workspace --all-features --no-run","cargo test --workspace --all-features --doc -- --list"]"#,
    "9486db831d5dd9a32ea5603b7a00e09a74a087be2b785b7068b4a8d6304def21"
);
semantics_case!(
    command_semicolon_suffix,
    "/cargo_inventory/enumeration_commands/0",
    "shell_suffix",
    "policy_invalid_command",
    r#""cargo test --workspace --all-features --no-run; echo pwned""#,
    "a161c0bd162fcbd6b39eebe4beb17f55076ce7bad3762352350419184a4571f7"
);
semantics_case!(
    command_dollar_substitution,
    "/cargo_inventory/enumeration_commands/0",
    "command_substitution",
    "policy_invalid_command",
    r#""cargo test --workspace --all-features --no-run $(echo pwned)""#,
    "ef94579711e662c22ea72517057552d0c16d4a953a548292ff03798c94ac7d2e"
);
semantics_case!(
    command_missing_workspace,
    "/cargo_inventory/enumeration_commands/0",
    "missing_flag",
    "policy_invalid_command",
    r#""cargo test --all-features --no-run""#,
    "4b314782a417b912f169f61ab6cb19ab7cd95ebda5298a6b2733c2dfdb7ab9d9"
);
semantics_case!(
    command_missing_all_features,
    "/cargo_inventory/enumeration_commands/0",
    "missing_flag",
    "policy_invalid_command",
    r#""cargo test --workspace --no-run""#,
    "2e42738f3f43a798a31af79f5f5f4fd4393a2386a52a727029e5c5f3e971ef26"
);
semantics_case!(
    command_ignored_filter,
    "/cargo_inventory/enumeration_commands/1",
    "test_filter",
    "policy_invalid_command",
    r#""cargo test --workspace --all-features -- --list --ignored""#,
    "75408f9b5d0b1385df4eaf38f7b8d4c39b3b4dfa361610607325f0d156ccdce1"
);
semantics_case!(
    command_whitespace_normalization,
    "/cargo_inventory/enumeration_commands/0",
    "whitespace",
    "policy_invalid_command",
    r#""cargo  test --workspace --all-features --no-run""#,
    "8501428e9fbe69814a6b6cc625ff9973d6293226cc354bd49724c44b5b45d42e"
);
semantics_case!(
    command_unicode_dash,
    "/cargo_inventory/enumeration_commands/0",
    "unicode_dash",
    "policy_invalid_command",
    r#""cargo test \u2013-workspace --all-features --no-run""#,
    "5481568fdbb1c0bc285cfe16a789daf48f4e42bec5d58345f9dd5e6aea4f4eca"
);
semantics_case!(
    command_flag_order,
    "/cargo_inventory/enumeration_commands/0",
    "flag_order",
    "policy_invalid_command",
    r#""cargo test --all-features --workspace --no-run""#,
    "24534e0168f2ddc8af852b4b9bb20864f56a0f0e608b24760c0c46c649468b61"
);
semantics_case!(
    extension_case_variant,
    "/marker_scan/extensions/0",
    "case_variant",
    "policy_invalid_entry",
    r#"".RS""#,
    "435244f50dce6bdb92a8a88e02715a1e402d1d1db9b118fa48f16a33dfd4b25e"
);
semantics_case!(
    extension_missing_dot,
    "/marker_scan/extensions/0",
    "missing_dot",
    "policy_invalid_entry",
    r#""rs""#,
    "f961413b990558f858e35bc8b07f3aebc66df8d06a0f977d30e5f3fafa0784dc"
);
semantics_case!(
    extension_trailing_whitespace,
    "/marker_scan/extensions/0",
    "trailing_whitespace",
    "policy_invalid_entry",
    r#"".rs ""#,
    "481e7d843aa77cc308fc0fd67e9ed499eb45d4d878243186181b97affd5273b5"
);
semantics_case!(
    extension_fullwidth_dot,
    "/marker_scan/extensions/0",
    "unicode_confusable",
    "policy_invalid_entry",
    r#""\uff0ers""#,
    "f28fd8ae0f0767a5c79b39d40d8955fbcf2ff82ba62a1c60e10d8127970c25c5"
);
semantics_case!(
    extension_nul_suffix,
    "/marker_scan/extensions/0",
    "nul_suffix",
    "policy_invalid_entry",
    r#"".rs\u0000""#,
    "9096ff94763a5c95254e523e15f46d9d5a9eb7afcec5e11764c595495c302935"
);
// BEGIN BASELINE SOURCE-BINDING CONTROLS
fn binding_pair(bytes: &[u8]) -> (TempDir, TempDir) {
    let source = TempDir::new().expect("BASELINE_CONTROL_FAILURE source tempdir");
    let scratch = TempDir::new().expect("BASELINE_CONTROL_FAILURE scratch tempdir");
    for repo in [source.path(), scratch.path()] {
        git(repo, &["init"]);
        git(
            repo,
            &["config", "user.email", "baseline-binding@example.invalid"],
        );
        git(repo, &["config", "user.name", "Baseline Binding Control"]);
        write_file(repo, BASELINE_PATH, bytes);
        git(repo, &["add", "--all"]);
        git(repo, &["commit", "-m", "bind baseline object"]);
    }
    (source, scratch)
}

fn legacy_binding_bytes() -> Vec<u8> {
    let bytes = git(
        &repo_root(),
        &[
            "show",
            "44839365ca63f350d0cfdd8247964407526e261d:tests/test-custody-baseline.json",
        ],
    )
    .stdout;
    assert_eq!(sha256_bytes(&bytes), LEGACY_BASELINE_SHA256);
    bytes
}

fn v2_binding_bytes() -> Vec<u8> {
    serde_json::to_vec(&json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "policy_path": POLICY_PATH,
        "policy_sha256": POLICY_SHA256,
        "whole_files": {},
        "rust_regions": {},
        "executable_identities": [],
        "rustdoc_identities": []
    }))
    .expect("BASELINE_CONTROL_FAILURE v2 fixture serialization")
}

fn assert_binding_rejected(source: &Path, scratch: &Path, reason: &str) {
    let panic = std::panic::catch_unwind(|| assert_baseline_bytes_bound_from(source, scratch))
        .expect_err("BASELINE_CONTROL_FAILURE invalid baseline unexpectedly accepted");
    let message = panic
        .downcast_ref::<String>()
        .map(String::as_str)
        .or_else(|| panic.downcast_ref::<&str>().copied())
        .expect("BASELINE_CONTROL_FAILURE panic must carry stable text");
    assert!(
        message.contains(reason),
        "BASELINE_CONTROL_FAILURE wrong rejection: expected {reason:?}, got {message:?}"
    );
}

#[test]
fn baseline_binding_v1_exact_passes() {
    let (source, scratch) = binding_pair(&legacy_binding_bytes());
    assert_baseline_bytes_bound_from(source.path(), scratch.path());
}

#[test]
fn baseline_binding_v1_changed_object_rejected() {
    let mut bytes = legacy_binding_bytes();
    bytes.push(b'\n');
    let (source, scratch) = binding_pair(&bytes);
    assert_binding_rejected(
        source.path(),
        scratch.path(),
        "legacy v1 baseline Git object digest changed",
    );
}

#[test]
fn baseline_binding_v2_exact_passes() {
    let (source, scratch) = binding_pair(&v2_binding_bytes());
    assert_baseline_bytes_bound_from(source.path(), scratch.path());
}

#[test]
fn baseline_binding_scratch_worktree_tamper_rejected() {
    let (source, scratch) = binding_pair(&v2_binding_bytes());
    let mut bytes = v2_binding_bytes();
    bytes.push(b'\n');
    write_file(scratch.path(), BASELINE_PATH, bytes);
    assert_binding_rejected(
        source.path(),
        scratch.path(),
        "scratch repository must be clean",
    );
}

#[test]
fn baseline_binding_scratch_object_tamper_rejected() {
    let (source, scratch) = binding_pair(&v2_binding_bytes());
    let mut bytes = v2_binding_bytes();
    bytes.push(b'\n');
    write_file(scratch.path(), BASELINE_PATH, bytes);
    git(scratch.path(), &["add", "--all"]);
    git(scratch.path(), &["commit", "-m", "tamper scratch object"]);
    assert_binding_rejected(
        source.path(),
        scratch.path(),
        "baseline bytes differ across",
    );
}

#[test]
fn baseline_binding_source_worktree_tamper_rejected() {
    let (source, scratch) = binding_pair(&v2_binding_bytes());
    let mut bytes = v2_binding_bytes();
    bytes.push(b'\n');
    write_file(source.path(), BASELINE_PATH, &bytes);
    write_file(scratch.path(), BASELINE_PATH, bytes);
    git(scratch.path(), &["add", "--all"]);
    git(
        scratch.path(),
        &["commit", "-m", "match tampered source worktree"],
    );
    assert_binding_rejected(
        source.path(),
        scratch.path(),
        "baseline bytes differ across",
    );
}

#[test]
fn baseline_binding_missing_rejected() {
    let (source, scratch) = binding_pair(&v2_binding_bytes());
    fs::remove_file(source.path().join(BASELINE_PATH))
        .expect("BASELINE_CONTROL_FAILURE remove source copy");
    assert_binding_rejected(source.path(), scratch.path(), "missing baseline");
}

#[test]
fn baseline_binding_symlink_rejected() {
    use std::os::unix::fs::symlink;

    let (source, scratch) = binding_pair(&v2_binding_bytes());
    let baseline = source.path().join(BASELINE_PATH);
    let destination = source.path().join("baseline-target.json");
    fs::rename(&baseline, &destination).expect("BASELINE_CONTROL_FAILURE move source copy");
    symlink(&destination, &baseline).expect("BASELINE_CONTROL_FAILURE link source copy");
    assert_binding_rejected(source.path(), scratch.path(), "regular non-symlink file");
}

#[test]
fn baseline_binding_malformed_json_rejected() {
    let (source, scratch) = binding_pair(b"{");
    assert_binding_rejected(source.path(), scratch.path(), "baseline must be valid JSON");
}

#[test]
fn baseline_binding_unsupported_version_rejected() {
    let bytes = serde_json::to_vec(&json!({"schema_version": 3}))
        .expect("BASELINE_CONTROL_FAILURE unsupported version fixture");
    let (source, scratch) = binding_pair(&bytes);
    assert_binding_rejected(
        source.path(),
        scratch.path(),
        "unsupported baseline shape or schema version",
    );
}

#[test]
fn baseline_binding_v2_missing_source_sha_rejected() {
    let mut value: Value = serde_json::from_slice(&v2_binding_bytes())
        .expect("BASELINE_CONTROL_FAILURE v2 fixture must parse");
    value.as_object_mut().unwrap().remove("source_sha");
    let bytes = serde_json::to_vec(&value).expect("BASELINE_CONTROL_FAILURE missing SHA fixture");
    let (source, scratch) = binding_pair(&bytes);
    assert_binding_rejected(source.path(), scratch.path(), "v2 source_sha missing");
}
// END BASELINE SOURCE-BINDING CONTROLS
