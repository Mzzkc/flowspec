#![allow(non_snake_case)]

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

const CHECKER_TIMEOUT: Duration = Duration::from_secs(5);
const GIT_TIMEOUT: Duration = Duration::from_secs(10);
const POLICY_SHA256: &str = "c14829a4d7428166cbd9b6a74cff24fb884d52733b549459f0a8ea3c61acc0d1";
const POLICY_PATH: &str = "tests/test-custody-policy.json";

const FIELDS: &[(&str, &str)] = &[
    ("/schema", "string"),
    ("/schema_version", "integer"),
    ("/policy_version", "string"),
    ("/deny_unknown_fields", "boolean"),
    ("/marker_scan", "object"),
    ("/whole_file_roots", "array"),
    ("/whole_file_harness_paths", "array"),
    ("/mixed_rust_roots", "array"),
    ("/rust_region_rules", "object"),
    ("/rust_include_edges", "object"),
    ("/control_baseline", "object"),
    ("/cargo_inventory", "object"),
    ("/marker_scan/tracked_regular_utf8_source", "boolean"),
    ("/marker_scan/path_exclusions_allowed", "boolean"),
    ("/marker_scan/extensions", "array"),
    ("/rust_region_rules/cfg_test_predicate", "boolean"),
    ("/rust_region_rules/test_attributes", "array"),
    ("/rust_region_rules/policy_test_attributes", "array"),
    (
        "/rust_region_rules/external_modules_under_test_cfg",
        "boolean",
    ),
    ("/rust_region_rules/executable_rustdoc_fences", "boolean"),
    (
        "/rust_region_rules/local_macro_invocations_inside_custodied_regions",
        "boolean",
    ),
    (
        "/rust_region_rules/fail_on_unclassified_test_constructs",
        "boolean",
    ),
    (
        "/rust_include_edges/resolve_constant_literal_include_str",
        "boolean",
    ),
    (
        "/rust_include_edges/resolve_constant_literal_include_bytes",
        "boolean",
    ),
    (
        "/rust_include_edges/repository_contained_regular_files_only",
        "boolean",
    ),
    ("/rust_include_edges/unresolved_include_is_error", "boolean"),
    ("/control_baseline/path", "string"),
    (
        "/control_baseline/excluded_from_own_file_hash_map",
        "boolean",
    ),
    ("/control_baseline/self_exclusion_reason", "string"),
    ("/cargo_inventory/platform_contract", "object"),
    ("/cargo_inventory/enumeration_commands", "array"),
    ("/cargo_inventory/exact_identity_set_required", "boolean"),
    ("/cargo_inventory/target_qualified_identities", "boolean"),
    ("/cargo_inventory/platform_contract/os", "string"),
    ("/cargo_inventory/platform_contract/workspace", "boolean"),
    ("/cargo_inventory/platform_contract/all_features", "boolean"),
];

const SCOPES: &[&str] = &[
    "",
    "/marker_scan",
    "/rust_region_rules",
    "/rust_include_edges",
    "/control_baseline",
    "/cargo_inventory",
    "/cargo_inventory/platform_contract",
];

const ARRAYS: &[&str] = &[
    "/whole_file_roots",
    "/whole_file_harness_paths",
    "/mixed_rust_roots",
    "/marker_scan/extensions",
    "/rust_region_rules/test_attributes",
    "/rust_region_rules/policy_test_attributes",
    "/cargo_inventory/enumeration_commands",
];

const ELEMENT_WRONG: &[&str] = &["null", "boolean", "number", "array", "object"];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn source_checker_path() -> PathBuf {
    repo_root().join("scripts/check-test-custody.py")
}

fn source_policy_path() -> PathBuf {
    repo_root().join(POLICY_PATH)
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
        &["config", "user.email", "custody-schema@example.invalid"],
    );
    git(repo, &["config", "user.name", "Custody Schema RED"]);

    write_file(
        repo,
        "scripts/check-test-custody.py",
        fs::read(source_checker_path()).expect("source checker must be readable"),
    );
    write_file(
        repo,
        POLICY_PATH,
        fs::read(source_policy_path()).expect("source policy must be readable"),
    );

    commit_all(repo, "clean custody schema fixture");
    assert_clean_fixture(repo);
    temp
}

fn assert_clean_fixture(repo: &Path) {
    assert_eq!(
        git_text(repo, &["status", "--porcelain"]),
        "",
        "scratch repository must start clean"
    );
    assert_eq!(
        sha256_file(&repo.join("scripts/check-test-custody.py")),
        sha256_file(&source_checker_path()),
        "scratch checker must be byte-identical to current source checker"
    );
    assert_eq!(
        sha256_file(&repo.join(POLICY_PATH)),
        POLICY_SHA256,
        "scratch policy must be byte-identical to committed authority"
    );
    let tracked = git_text(repo, &["ls-files"]);
    for required in ["scripts/check-test-custody.py", POLICY_PATH] {
        assert!(
            tracked.lines().any(|line| line == required),
            "scratch repository must track {required}; tracked={tracked}"
        );
    }
}

fn run_check_markers(repo: &Path) -> Output {
    let mut command = Command::new("python3");
    command
        .current_dir(repo)
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .arg("--policy")
        .arg(POLICY_PATH)
        .arg("--check-markers");
    run_bounded(command, CHECKER_TIMEOUT, "custody checker marker check")
}

fn assert_valid_control(identity: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        0,
        "{identity}: valid policy control must exit 0, stdout={} stderr={}",
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
        "{identity}: valid policy marker control must keep stderr empty"
    );
}

fn policy_json(repo: &Path) -> Value {
    serde_json::from_str(
        &fs::read_to_string(repo.join(POLICY_PATH)).expect("policy must be readable"),
    )
    .expect("policy must parse as JSON")
}

fn write_policy_json(repo: &Path, policy: &Value) {
    let mut bytes = serde_json::to_vec_pretty(policy).expect("policy JSON must serialize");
    bytes.push(b'\n');
    write_file(repo, POLICY_PATH, bytes);
}

fn pointer_tokens(pointer: &str) -> Vec<String> {
    assert!(
        pointer.starts_with('/'),
        "mutation pointer must be absolute: {pointer}"
    );
    pointer
        .split('/')
        .skip(1)
        .map(|token| token.replace("~1", "/").replace("~0", "~"))
        .collect()
}

fn parent_mut<'a>(value: &'a mut Value, pointer: &str) -> (&'a mut Value, String) {
    let mut tokens = pointer_tokens(pointer);
    let last = tokens
        .pop()
        .unwrap_or_else(|| panic!("pointer must identify a field or item: {pointer}"));
    let mut cursor = value;
    for token in tokens {
        match cursor {
            Value::Object(map) => {
                cursor = map
                    .get_mut(&token)
                    .unwrap_or_else(|| panic!("missing object token {token} in {pointer}"));
            }
            Value::Array(items) => {
                let index = token
                    .parse::<usize>()
                    .unwrap_or_else(|_| panic!("invalid array token {token} in {pointer}"));
                cursor = items
                    .get_mut(index)
                    .unwrap_or_else(|| panic!("missing array token {token} in {pointer}"));
            }
            other => panic!("non-container parent at {pointer}: {other:?}"),
        }
    }
    (cursor, last)
}

fn remove_pointer(value: &mut Value, pointer: &str) {
    let (parent, key) = parent_mut(value, pointer);
    match parent {
        Value::Object(map) => {
            assert!(map.remove(&key).is_some(), "missing required key {pointer}");
        }
        Value::Array(items) => {
            let index = key
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("invalid array token {key} in {pointer}"));
            items.remove(index);
        }
        other => panic!("remove parent must be a container at {pointer}: {other:?}"),
    }
}

fn replace_pointer(value: &mut Value, pointer: &str, replacement: Value) {
    let target = value
        .pointer_mut(pointer)
        .unwrap_or_else(|| panic!("replacement target must exist: {pointer}"));
    *target = replacement;
}

fn insert_or_replace_pointer(value: &mut Value, pointer: &str, replacement: Value) {
    let (parent, key) = parent_mut(value, pointer);
    match parent {
        Value::Object(map) => {
            map.insert(key, replacement);
        }
        Value::Array(items) => {
            let index = key
                .parse::<usize>()
                .unwrap_or_else(|_| panic!("invalid array token {key} in {pointer}"));
            if index < items.len() {
                items[index] = replacement;
            } else {
                assert_eq!(
                    index,
                    items.len(),
                    "array item insertion must be at the next index for {pointer}"
                );
                items.insert(index, replacement);
            }
        }
        other => panic!("insert parent must be a container at {pointer}: {other:?}"),
    }
}

fn wrong_value(kind: &str) -> Value {
    match kind {
        "null" => Value::Null,
        "boolean" => json!(false),
        "number" => json!(7),
        "non_integer_number" => json!(1.5),
        "string" => json!("wrong-type"),
        "array" => json!(["wrong-type"]),
        "object" => json!({"wrong_type": true}),
        other => panic!("unknown wrong-value kind {other}"),
    }
}

fn apply_mutation(policy: &mut Value, pointer: &str, mutation: &str) {
    if mutation == "remove" {
        remove_pointer(policy, pointer);
    } else if mutation == "insert_unknown" {
        insert_or_replace_pointer(policy, pointer, json!("schema-red-unknown"));
    } else if let Some(kind) = mutation.strip_prefix("replace_with_") {
        replace_pointer(policy, pointer, wrong_value(kind));
    } else if let Some(kind) = mutation.strip_prefix("item_") {
        insert_or_replace_pointer(policy, pointer, wrong_value(kind));
    } else {
        match mutation {
            "bind_wrong_schema" => replace_pointer(policy, pointer, json!("flowspec.wrong_policy")),
            "bind_wrong_version" => replace_pointer(policy, pointer, json!(2)),
            "bind_unknown_fields_false" => replace_pointer(policy, pointer, json!(false)),
            other => panic!("unknown schema mutation {other}"),
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
                    (Some(l), Some(r)) => collect_diffs(l, r, &child, diffs),
                    _ => diffs.push(child),
                }
            }
        }
        (Value::Array(left), Value::Array(right)) => {
            for index in 0..left.len().max(right.len()) {
                let child = join_pointer(pointer, &index.to_string());
                match (left.get(index), right.get(index)) {
                    (Some(l), Some(r)) => collect_diffs(l, r, &child, diffs),
                    _ => diffs.push(child),
                }
            }
        }
        _ => diffs.push(pointer.to_owned()),
    }
}

fn assert_single_diff(identity: &str, before: &Value, after: &Value, pointer: &str) {
    let mut diffs = Vec::new();
    collect_diffs(before, after, "", &mut diffs);
    assert_eq!(
        diffs,
        vec![pointer.to_owned()],
        "{identity}: structured mutation diff must be exactly {pointer}"
    );
}

fn assert_future_schema_rejection(identity: &str, pointer: &str, reason: &str, output: &Output) {
    assert_eq!(
        exit_code(output),
        1,
        "{identity}: future schema rejection must exit 1 for {reason} at {pointer}, stdout={} stderr={}",
        stdout(output),
        stderr(output)
    );
    assert_eq!(
        stdout(output),
        "",
        "{identity}: future schema rejection must keep stdout empty"
    );
    assert_eq!(
        stderr(output),
        format!("ERROR: {reason}: {pointer}\n"),
        "{identity}: future schema rejection must report exactly the stable reason and pointer"
    );
}

fn run_schema_case(identity: &str, pointer: &str, mutation: &str, reason: &str) {
    let temp = init_clean_repo();
    let repo = temp.path();
    assert_eq!(
        sha256_file(&repo.join(POLICY_PATH)),
        POLICY_SHA256,
        "{identity}: valid policy SHA must be proved before control"
    );
    let control = run_check_markers(repo);
    assert_valid_control(identity, &control);

    let before = policy_json(repo);
    let mut after = before.clone();
    apply_mutation(&mut after, pointer, mutation);
    assert_single_diff(identity, &before, &after, pointer);
    write_policy_json(repo, &after);
    assert_eq!(
        policy_json(repo),
        after,
        "{identity}: written mutated policy must parse back identically"
    );

    let observed = run_check_markers(repo);
    println!(
        "POLICY_SCHEMA_RED case_id={identity} pointer={pointer} mutation={mutation} expected_reason={reason} control_exit={} observed_exit={}",
        exit_code(&control),
        exit_code(&observed)
    );
    assert_future_schema_rejection(identity, pointer, reason, &observed);
}

fn expected_wrong_type_count(kind: &str) -> usize {
    match kind {
        "string" | "boolean" | "array" | "object" => 5,
        "integer" => 6,
        other => panic!("unknown field kind {other}"),
    }
}

fn assert_policy_value_type(pointer: &str, kind: &str, value: &Value) {
    let found = value
        .pointer(pointer)
        .unwrap_or_else(|| panic!("canonical policy must contain {pointer}"));
    let valid = match kind {
        "string" => found.is_string(),
        "integer" => found.as_i64().is_some(),
        "boolean" => found.is_boolean(),
        "array" => found.is_array(),
        "object" => found.is_object(),
        other => panic!("unknown field kind {other}"),
    };
    assert!(
        valid,
        "canonical policy {pointer} must be {kind}, got {found:?}"
    );
}

#[test]
fn policy_schema_case_ledger_is_exact() {
    let policy_path = source_policy_path();
    assert_eq!(sha256_file(&policy_path), POLICY_SHA256);
    let policy: Value = serde_json::from_str(
        &fs::read_to_string(policy_path).expect("canonical policy must be readable"),
    )
    .expect("canonical policy must parse as JSON");

    assert_eq!(FIELDS.len(), 36, "field ledger must contain 36 edges");
    assert_eq!(
        SCOPES.len(),
        7,
        "scope ledger must contain 7 closed objects"
    );
    assert_eq!(ARRAYS.len(), 7, "array ledger must contain 7 string arrays");
    for (pointer, kind) in FIELDS {
        assert_policy_value_type(pointer, kind, &policy);
    }
    for scope in SCOPES {
        let pointer = if scope.is_empty() { "/" } else { scope };
        let object = if scope.is_empty() {
            Some(&policy)
        } else {
            policy.pointer(scope)
        };
        assert!(
            object.is_some_and(Value::is_object),
            "closed scope {pointer} must exist as an object"
        );
    }
    for pointer in ARRAYS {
        assert!(
            policy.pointer(pointer).is_some_and(Value::is_array),
            "array pointer {pointer} must exist as an array"
        );
    }
    assert_eq!(
        policy.pointer("/schema"),
        Some(&json!("flowspec.test_custody_policy"))
    );
    assert_eq!(policy.pointer("/schema_version"), Some(&json!(1)));
    assert_eq!(policy.pointer("/deny_unknown_fields"), Some(&json!(true)));

    let missing_required = FIELDS.len();
    let invalid_type = FIELDS
        .iter()
        .map(|(_, kind)| expected_wrong_type_count(kind))
        .sum::<usize>();
    let unknown_field = SCOPES.len();
    let invalid_array_element_type = ARRAYS.len() * ELEMENT_WRONG.len();
    let invalid_binding = 3;
    assert_eq!(missing_required, 36);
    assert_eq!(invalid_type, 181);
    assert_eq!(unknown_field, 7);
    assert_eq!(invalid_array_element_type, 35);
    assert_eq!(invalid_binding, 3);
    assert_eq!(
        missing_required
            + invalid_type
            + unknown_field
            + invalid_array_element_type
            + invalid_binding,
        262
    );
}

macro_rules! schema_case {
    ($name:ident, $pointer:expr, $mutation:expr, $reason:expr) => {
        #[test]
        fn $name() {
            run_schema_case(stringify!($name), $pointer, $mutation, $reason);
        }
    };
}

schema_case!(
    policy_missing_required__schema,
    "/schema",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__schema_version,
    "/schema_version",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__policy_version,
    "/policy_version",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__deny_unknown_fields,
    "/deny_unknown_fields",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__marker_scan,
    "/marker_scan",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__whole_file_roots,
    "/whole_file_roots",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__whole_file_harness_paths,
    "/whole_file_harness_paths",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__mixed_rust_roots,
    "/mixed_rust_roots",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules,
    "/rust_region_rules",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_include_edges,
    "/rust_include_edges",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__control_baseline,
    "/control_baseline",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory,
    "/cargo_inventory",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__marker_scan__tracked_regular_utf8_source,
    "/marker_scan/tracked_regular_utf8_source",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__marker_scan__path_exclusions_allowed,
    "/marker_scan/path_exclusions_allowed",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__marker_scan__extensions,
    "/marker_scan/extensions",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__cfg_test_predicate,
    "/rust_region_rules/cfg_test_predicate",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__test_attributes,
    "/rust_region_rules/test_attributes",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__policy_test_attributes,
    "/rust_region_rules/policy_test_attributes",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__external_modules_under_test_cfg,
    "/rust_region_rules/external_modules_under_test_cfg",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__executable_rustdoc_fences,
    "/rust_region_rules/executable_rustdoc_fences",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__local_macro_invocations_inside_custodied_regions,
    "/rust_region_rules/local_macro_invocations_inside_custodied_regions",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_region_rules__fail_on_unclassified_test_constructs,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_include_edges__resolve_constant_literal_include_str,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_include_edges__resolve_constant_literal_include_bytes,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_include_edges__repository_contained_regular_files_only,
    "/rust_include_edges/repository_contained_regular_files_only",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__rust_include_edges__unresolved_include_is_error,
    "/rust_include_edges/unresolved_include_is_error",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__control_baseline__path,
    "/control_baseline/path",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__control_baseline__excluded_from_own_file_hash_map,
    "/control_baseline/excluded_from_own_file_hash_map",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__control_baseline__self_exclusion_reason,
    "/control_baseline/self_exclusion_reason",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__platform_contract,
    "/cargo_inventory/platform_contract",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__enumeration_commands,
    "/cargo_inventory/enumeration_commands",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__exact_identity_set_required,
    "/cargo_inventory/exact_identity_set_required",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__target_qualified_identities,
    "/cargo_inventory/target_qualified_identities",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__platform_contract__os,
    "/cargo_inventory/platform_contract/os",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__platform_contract__workspace,
    "/cargo_inventory/platform_contract/workspace",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_missing_required__cargo_inventory__platform_contract__all_features,
    "/cargo_inventory/platform_contract/all_features",
    "remove",
    "policy_missing_required"
);
schema_case!(
    policy_invalid_type__schema__as_null,
    "/schema",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema__as_boolean,
    "/schema",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema__as_number,
    "/schema",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema__as_array,
    "/schema",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema__as_object,
    "/schema",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_null,
    "/schema_version",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_boolean,
    "/schema_version",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_non_integer_number,
    "/schema_version",
    "replace_with_non_integer_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_string,
    "/schema_version",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_array,
    "/schema_version",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__schema_version__as_object,
    "/schema_version",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__policy_version__as_null,
    "/policy_version",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__policy_version__as_boolean,
    "/policy_version",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__policy_version__as_number,
    "/policy_version",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__policy_version__as_array,
    "/policy_version",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__policy_version__as_object,
    "/policy_version",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__deny_unknown_fields__as_null,
    "/deny_unknown_fields",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__deny_unknown_fields__as_number,
    "/deny_unknown_fields",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__deny_unknown_fields__as_string,
    "/deny_unknown_fields",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__deny_unknown_fields__as_array,
    "/deny_unknown_fields",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__deny_unknown_fields__as_object,
    "/deny_unknown_fields",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__as_null,
    "/marker_scan",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__as_boolean,
    "/marker_scan",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__as_number,
    "/marker_scan",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__as_string,
    "/marker_scan",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__as_array,
    "/marker_scan",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__as_null,
    "/whole_file_roots",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__as_boolean,
    "/whole_file_roots",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__as_number,
    "/whole_file_roots",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__as_string,
    "/whole_file_roots",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__as_object,
    "/whole_file_roots",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__as_null,
    "/whole_file_harness_paths",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__as_boolean,
    "/whole_file_harness_paths",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__as_number,
    "/whole_file_harness_paths",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__as_string,
    "/whole_file_harness_paths",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__as_object,
    "/whole_file_harness_paths",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__as_null,
    "/mixed_rust_roots",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__as_boolean,
    "/mixed_rust_roots",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__as_number,
    "/mixed_rust_roots",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__as_string,
    "/mixed_rust_roots",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__as_object,
    "/mixed_rust_roots",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__as_null,
    "/rust_region_rules",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__as_boolean,
    "/rust_region_rules",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__as_number,
    "/rust_region_rules",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__as_string,
    "/rust_region_rules",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__as_array,
    "/rust_region_rules",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__as_null,
    "/rust_include_edges",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__as_boolean,
    "/rust_include_edges",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__as_number,
    "/rust_include_edges",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__as_string,
    "/rust_include_edges",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__as_array,
    "/rust_include_edges",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__as_null,
    "/control_baseline",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__as_boolean,
    "/control_baseline",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__as_number,
    "/control_baseline",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__as_string,
    "/control_baseline",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__as_array,
    "/control_baseline",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__as_null,
    "/cargo_inventory",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__as_boolean,
    "/cargo_inventory",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__as_number,
    "/cargo_inventory",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__as_string,
    "/cargo_inventory",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__as_array,
    "/cargo_inventory",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__tracked_regular_utf8_source__as_null,
    "/marker_scan/tracked_regular_utf8_source",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__tracked_regular_utf8_source__as_number,
    "/marker_scan/tracked_regular_utf8_source",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__tracked_regular_utf8_source__as_string,
    "/marker_scan/tracked_regular_utf8_source",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__tracked_regular_utf8_source__as_array,
    "/marker_scan/tracked_regular_utf8_source",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__tracked_regular_utf8_source__as_object,
    "/marker_scan/tracked_regular_utf8_source",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__path_exclusions_allowed__as_null,
    "/marker_scan/path_exclusions_allowed",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__path_exclusions_allowed__as_number,
    "/marker_scan/path_exclusions_allowed",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__path_exclusions_allowed__as_string,
    "/marker_scan/path_exclusions_allowed",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__path_exclusions_allowed__as_array,
    "/marker_scan/path_exclusions_allowed",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__path_exclusions_allowed__as_object,
    "/marker_scan/path_exclusions_allowed",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__as_null,
    "/marker_scan/extensions",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__as_boolean,
    "/marker_scan/extensions",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__as_number,
    "/marker_scan/extensions",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__as_string,
    "/marker_scan/extensions",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__as_object,
    "/marker_scan/extensions",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__cfg_test_predicate__as_null,
    "/rust_region_rules/cfg_test_predicate",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__cfg_test_predicate__as_number,
    "/rust_region_rules/cfg_test_predicate",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__cfg_test_predicate__as_string,
    "/rust_region_rules/cfg_test_predicate",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__cfg_test_predicate__as_array,
    "/rust_region_rules/cfg_test_predicate",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__cfg_test_predicate__as_object,
    "/rust_region_rules/cfg_test_predicate",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__as_null,
    "/rust_region_rules/test_attributes",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__as_boolean,
    "/rust_region_rules/test_attributes",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__as_number,
    "/rust_region_rules/test_attributes",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__as_string,
    "/rust_region_rules/test_attributes",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__as_object,
    "/rust_region_rules/test_attributes",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__as_null,
    "/rust_region_rules/policy_test_attributes",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__as_boolean,
    "/rust_region_rules/policy_test_attributes",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__as_number,
    "/rust_region_rules/policy_test_attributes",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__as_string,
    "/rust_region_rules/policy_test_attributes",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__as_object,
    "/rust_region_rules/policy_test_attributes",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__external_modules_under_test_cfg__as_null,
    "/rust_region_rules/external_modules_under_test_cfg",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__external_modules_under_test_cfg__as_number,
    "/rust_region_rules/external_modules_under_test_cfg",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__external_modules_under_test_cfg__as_string,
    "/rust_region_rules/external_modules_under_test_cfg",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__external_modules_under_test_cfg__as_array,
    "/rust_region_rules/external_modules_under_test_cfg",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__external_modules_under_test_cfg__as_object,
    "/rust_region_rules/external_modules_under_test_cfg",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__executable_rustdoc_fences__as_null,
    "/rust_region_rules/executable_rustdoc_fences",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__executable_rustdoc_fences__as_number,
    "/rust_region_rules/executable_rustdoc_fences",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__executable_rustdoc_fences__as_string,
    "/rust_region_rules/executable_rustdoc_fences",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__executable_rustdoc_fences__as_array,
    "/rust_region_rules/executable_rustdoc_fences",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__executable_rustdoc_fences__as_object,
    "/rust_region_rules/executable_rustdoc_fences",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(policy_invalid_type__rust_region_rules__local_macro_invocations_inside_custodied_regions__as_null, "/rust_region_rules/local_macro_invocations_inside_custodied_regions", "replace_with_null", "policy_invalid_type");
schema_case!(policy_invalid_type__rust_region_rules__local_macro_invocations_inside_custodied_regions__as_number, "/rust_region_rules/local_macro_invocations_inside_custodied_regions", "replace_with_number", "policy_invalid_type");
schema_case!(policy_invalid_type__rust_region_rules__local_macro_invocations_inside_custodied_regions__as_string, "/rust_region_rules/local_macro_invocations_inside_custodied_regions", "replace_with_string", "policy_invalid_type");
schema_case!(policy_invalid_type__rust_region_rules__local_macro_invocations_inside_custodied_regions__as_array, "/rust_region_rules/local_macro_invocations_inside_custodied_regions", "replace_with_array", "policy_invalid_type");
schema_case!(policy_invalid_type__rust_region_rules__local_macro_invocations_inside_custodied_regions__as_object, "/rust_region_rules/local_macro_invocations_inside_custodied_regions", "replace_with_object", "policy_invalid_type");
schema_case!(
    policy_invalid_type__rust_region_rules__fail_on_unclassified_test_constructs__as_null,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__fail_on_unclassified_test_constructs__as_number,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__fail_on_unclassified_test_constructs__as_string,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__fail_on_unclassified_test_constructs__as_array,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__fail_on_unclassified_test_constructs__as_object,
    "/rust_region_rules/fail_on_unclassified_test_constructs",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_str__as_null,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_str__as_number,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_str__as_string,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_str__as_array,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_str__as_object,
    "/rust_include_edges/resolve_constant_literal_include_str",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_bytes__as_null,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_bytes__as_number,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_bytes__as_string,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_bytes__as_array,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__resolve_constant_literal_include_bytes__as_object,
    "/rust_include_edges/resolve_constant_literal_include_bytes",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__repository_contained_regular_files_only__as_null,
    "/rust_include_edges/repository_contained_regular_files_only",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__repository_contained_regular_files_only__as_number,
    "/rust_include_edges/repository_contained_regular_files_only",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__repository_contained_regular_files_only__as_string,
    "/rust_include_edges/repository_contained_regular_files_only",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__repository_contained_regular_files_only__as_array,
    "/rust_include_edges/repository_contained_regular_files_only",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__repository_contained_regular_files_only__as_object,
    "/rust_include_edges/repository_contained_regular_files_only",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__unresolved_include_is_error__as_null,
    "/rust_include_edges/unresolved_include_is_error",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__unresolved_include_is_error__as_number,
    "/rust_include_edges/unresolved_include_is_error",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__unresolved_include_is_error__as_string,
    "/rust_include_edges/unresolved_include_is_error",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__unresolved_include_is_error__as_array,
    "/rust_include_edges/unresolved_include_is_error",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_include_edges__unresolved_include_is_error__as_object,
    "/rust_include_edges/unresolved_include_is_error",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__path__as_null,
    "/control_baseline/path",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__path__as_boolean,
    "/control_baseline/path",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__path__as_number,
    "/control_baseline/path",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__path__as_array,
    "/control_baseline/path",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__path__as_object,
    "/control_baseline/path",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__excluded_from_own_file_hash_map__as_null,
    "/control_baseline/excluded_from_own_file_hash_map",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__excluded_from_own_file_hash_map__as_number,
    "/control_baseline/excluded_from_own_file_hash_map",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__excluded_from_own_file_hash_map__as_string,
    "/control_baseline/excluded_from_own_file_hash_map",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__excluded_from_own_file_hash_map__as_array,
    "/control_baseline/excluded_from_own_file_hash_map",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__excluded_from_own_file_hash_map__as_object,
    "/control_baseline/excluded_from_own_file_hash_map",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__self_exclusion_reason__as_null,
    "/control_baseline/self_exclusion_reason",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__self_exclusion_reason__as_boolean,
    "/control_baseline/self_exclusion_reason",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__self_exclusion_reason__as_number,
    "/control_baseline/self_exclusion_reason",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__self_exclusion_reason__as_array,
    "/control_baseline/self_exclusion_reason",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__control_baseline__self_exclusion_reason__as_object,
    "/control_baseline/self_exclusion_reason",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__as_null,
    "/cargo_inventory/platform_contract",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__as_boolean,
    "/cargo_inventory/platform_contract",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__as_number,
    "/cargo_inventory/platform_contract",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__as_string,
    "/cargo_inventory/platform_contract",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__as_array,
    "/cargo_inventory/platform_contract",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__as_null,
    "/cargo_inventory/enumeration_commands",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__as_boolean,
    "/cargo_inventory/enumeration_commands",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__as_number,
    "/cargo_inventory/enumeration_commands",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__as_string,
    "/cargo_inventory/enumeration_commands",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__as_object,
    "/cargo_inventory/enumeration_commands",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__exact_identity_set_required__as_null,
    "/cargo_inventory/exact_identity_set_required",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__exact_identity_set_required__as_number,
    "/cargo_inventory/exact_identity_set_required",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__exact_identity_set_required__as_string,
    "/cargo_inventory/exact_identity_set_required",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__exact_identity_set_required__as_array,
    "/cargo_inventory/exact_identity_set_required",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__exact_identity_set_required__as_object,
    "/cargo_inventory/exact_identity_set_required",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__target_qualified_identities__as_null,
    "/cargo_inventory/target_qualified_identities",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__target_qualified_identities__as_number,
    "/cargo_inventory/target_qualified_identities",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__target_qualified_identities__as_string,
    "/cargo_inventory/target_qualified_identities",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__target_qualified_identities__as_array,
    "/cargo_inventory/target_qualified_identities",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__target_qualified_identities__as_object,
    "/cargo_inventory/target_qualified_identities",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__os__as_null,
    "/cargo_inventory/platform_contract/os",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__os__as_boolean,
    "/cargo_inventory/platform_contract/os",
    "replace_with_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__os__as_number,
    "/cargo_inventory/platform_contract/os",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__os__as_array,
    "/cargo_inventory/platform_contract/os",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__os__as_object,
    "/cargo_inventory/platform_contract/os",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__workspace__as_null,
    "/cargo_inventory/platform_contract/workspace",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__workspace__as_number,
    "/cargo_inventory/platform_contract/workspace",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__workspace__as_string,
    "/cargo_inventory/platform_contract/workspace",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__workspace__as_array,
    "/cargo_inventory/platform_contract/workspace",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__workspace__as_object,
    "/cargo_inventory/platform_contract/workspace",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__all_features__as_null,
    "/cargo_inventory/platform_contract/all_features",
    "replace_with_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__all_features__as_number,
    "/cargo_inventory/platform_contract/all_features",
    "replace_with_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__all_features__as_string,
    "/cargo_inventory/platform_contract/all_features",
    "replace_with_string",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__all_features__as_array,
    "/cargo_inventory/platform_contract/all_features",
    "replace_with_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__platform_contract__all_features__as_object,
    "/cargo_inventory/platform_contract/all_features",
    "replace_with_object",
    "policy_invalid_type"
);
schema_case!(
    policy_unknown_field__root,
    "/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__marker_scan,
    "/marker_scan/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__rust_region_rules,
    "/rust_region_rules/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__rust_include_edges,
    "/rust_include_edges/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__control_baseline,
    "/control_baseline/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__cargo_inventory,
    "/cargo_inventory/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_unknown_field__cargo_inventory__platform_contract,
    "/cargo_inventory/platform_contract/unknown",
    "insert_unknown",
    "policy_unknown_field"
);
schema_case!(
    policy_invalid_type__whole_file_roots__item_as_null,
    "/whole_file_roots/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__item_as_boolean,
    "/whole_file_roots/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__item_as_number,
    "/whole_file_roots/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__item_as_array,
    "/whole_file_roots/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_roots__item_as_object,
    "/whole_file_roots/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__item_as_null,
    "/whole_file_harness_paths/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__item_as_boolean,
    "/whole_file_harness_paths/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__item_as_number,
    "/whole_file_harness_paths/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__item_as_array,
    "/whole_file_harness_paths/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__whole_file_harness_paths__item_as_object,
    "/whole_file_harness_paths/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__item_as_null,
    "/mixed_rust_roots/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__item_as_boolean,
    "/mixed_rust_roots/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__item_as_number,
    "/mixed_rust_roots/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__item_as_array,
    "/mixed_rust_roots/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__mixed_rust_roots__item_as_object,
    "/mixed_rust_roots/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__item_as_null,
    "/marker_scan/extensions/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__item_as_boolean,
    "/marker_scan/extensions/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__item_as_number,
    "/marker_scan/extensions/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__item_as_array,
    "/marker_scan/extensions/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__marker_scan__extensions__item_as_object,
    "/marker_scan/extensions/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__item_as_null,
    "/rust_region_rules/test_attributes/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__item_as_boolean,
    "/rust_region_rules/test_attributes/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__item_as_number,
    "/rust_region_rules/test_attributes/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__item_as_array,
    "/rust_region_rules/test_attributes/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__test_attributes__item_as_object,
    "/rust_region_rules/test_attributes/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__item_as_null,
    "/rust_region_rules/policy_test_attributes/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__item_as_boolean,
    "/rust_region_rules/policy_test_attributes/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__item_as_number,
    "/rust_region_rules/policy_test_attributes/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__item_as_array,
    "/rust_region_rules/policy_test_attributes/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__rust_region_rules__policy_test_attributes__item_as_object,
    "/rust_region_rules/policy_test_attributes/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__item_as_null,
    "/cargo_inventory/enumeration_commands/0",
    "item_null",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__item_as_boolean,
    "/cargo_inventory/enumeration_commands/0",
    "item_boolean",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__item_as_number,
    "/cargo_inventory/enumeration_commands/0",
    "item_number",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__item_as_array,
    "/cargo_inventory/enumeration_commands/0",
    "item_array",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_type__cargo_inventory__enumeration_commands__item_as_object,
    "/cargo_inventory/enumeration_commands/0",
    "item_object",
    "policy_invalid_type"
);
schema_case!(
    policy_invalid_binding__schema,
    "/schema",
    "bind_wrong_schema",
    "policy_invalid_binding"
);
schema_case!(
    policy_invalid_binding__schema_version,
    "/schema_version",
    "bind_wrong_version",
    "policy_invalid_binding"
);
schema_case!(
    policy_invalid_binding__deny_unknown_fields,
    "/deny_unknown_fields",
    "bind_unknown_fields_false",
    "policy_invalid_binding"
);

// BEGIN ACTION CONTROLS
#[test]
fn action_valid_policy_succeeds_quietly() {
    let temp = init_clean_repo();
    let output = run_check_markers(temp.path());
    assert_valid_control("action_valid_policy_succeeds_quietly", &output);
}

#[test]
fn action_live_marker_rejected() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let marker = ["#[", "ignore]\n"].concat();
    write_file(repo, "tests/live-marker.rs", marker);
    commit_all(repo, "committed live Rust marker");
    let output = run_check_markers(repo);
    assert_eq!(exit_code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert_eq!(
        stderr(&output),
        "tests/live-marker.rs:1: rust_ignore: #[ignore]\n"
    );
}

#[test]
fn action_schema_invalid_policy_precedes_marker() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let marker = ["#[", "ignore]\n"].concat();
    write_file(repo, "tests/live-marker.rs", marker);
    commit_all(repo, "committed live Rust marker");
    let mut policy = policy_json(repo);
    policy
        .as_object_mut()
        .expect("policy object")
        .remove("schema");
    write_policy_json(repo, &policy);
    let output = run_check_markers(repo);
    assert_future_schema_rejection(
        "action_schema_invalid_policy_precedes_marker",
        "/schema",
        "policy_missing_required",
        &output,
    );
}

#[test]
fn action_self_test_valid_policy_rejected() {
    let temp = init_clean_repo();
    let repo = temp.path();
    let mut command = Command::new("python3");
    command
        .current_dir(repo)
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .arg("--policy")
        .arg(POLICY_PATH)
        .arg("--self-test");
    let output = run_bounded(command, CHECKER_TIMEOUT, "valid-policy self-test rejection");
    assert_ne!(exit_code(&output), 0, "valid-policy self-test must fail");
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "ERROR: self_test_noop_rejected\n");
}

#[test]
fn action_self_test_without_policy_requires_policy() {
    let temp = init_clean_repo();
    let mut command = Command::new("python3");
    command
        .current_dir(temp.path())
        .arg("scripts/check-test-custody.py")
        .arg("--root")
        .arg(".")
        .arg("--self-test");
    let output = run_bounded(command, CHECKER_TIMEOUT, "no-policy self-test rejection");
    assert_eq!(exit_code(&output), 2);
    assert_eq!(stdout(&output), "");
    assert_eq!(stderr(&output), "ERROR: policy_required\n");
}
// END ACTION CONTROLS
