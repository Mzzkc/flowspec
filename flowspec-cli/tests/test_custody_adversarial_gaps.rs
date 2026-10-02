use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, FileTypeExt, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};
use tempfile::TempDir;

fn checker_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/check-test-custody.py")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn policy_file_path() -> PathBuf {
    repo_root().join("tests/test-custody-policy.json")
}

fn init_repo() -> TempDir {
    let temp = TempDir::new().expect("fixture repository must be created");
    let repo = temp.path();
    git(repo, &["init"]);
    git(repo, &["config", "user.email", "custody@example.invalid"]);
    git(repo, &["config", "user.name", "Custody Adversary"]);
    temp
}

fn git(repo: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git must execute in fixture repository");
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
    fs::create_dir_all(path.parent().expect("fixture path must have parent"))
        .expect("fixture parent must be created");
    fs::write(path, content).expect("fixture file must be written");
}

fn commit_all(repo: &Path, message: &str) -> String {
    git(repo, &["add", "--all"]);
    git(repo, &["commit", "-m", message]);
    git_text(repo, &["rev-parse", "HEAD"])
}

fn run_checker(repo: &Path, args: &[&str]) -> Output {
    Command::new("python3")
        .arg(checker_path())
        .arg("--root")
        .arg(repo)
        .args(args)
        .current_dir(repo)
        .output()
        .expect("custody checker must execute")
}

fn run_checker_with_timeout(
    repo: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = Command::new("python3")
        .arg(checker_path())
        .arg("--root")
        .arg(repo)
        .args(args)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("custody checker must spawn");
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .expect("custody checker status must be observable")
            .is_some()
        {
            return child
                .wait_with_output()
                .map_err(|error| format!("failed to collect checker output: {error}"));
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("checker exceeded {} ms", timeout.as_millis()));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn assert_success(output: &Output, contract: &str) {
    assert!(
        output.status.success(),
        "{contract}: expected success, stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn assert_failure_evidence(output: &Output, contract: &str, evidence: &[&str]) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "{contract}: expected failure, stdout={stdout} stderr={stderr}"
    );
    for expected in evidence {
        assert!(
            stderr.contains(expected),
            "{contract}: missing evidence {expected:?}, stderr={stderr}"
        );
    }
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("JSON file must be readable"))
        .expect("JSON must parse")
}

fn marker(parts: &[&str]) -> String {
    parts.concat()
}

fn copy_policy_and_harness(repo: &Path) {
    let policy_src = policy_file_path();
    write_file(
        repo,
        "tests/test-custody-policy.json",
        fs::read(&policy_src).unwrap(),
    );
    write_file(
        repo,
        "scripts/check-test-custody.py",
        fs::read(checker_path()).unwrap(),
    );
    write_file(repo, "scripts/flowspec-oracle.py", b"# dummy oracle\n");
    write_file(
        repo,
        "scripts/validate-oracle-artifacts.py",
        b"# dummy validator\n",
    );
}

fn setup_inventory_repo() -> TempDir {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(
        repo,
        "Cargo.toml",
        "[workspace]\nmembers = [\"fixture\"]\nresolver = \"2\"\n",
    );
    write_file(repo, ".gitignore", "target/\n");
    write_file(
        repo,
        "fixture/Cargo.toml",
        "[package]\nname = \"custody-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    );
    write_file(
        repo,
        "fixture/src/lib.rs",
        "pub fn product() -> bool { true }\n",
    );
    write_file(
        repo,
        "fixture/tests/inventory.rs",
        "#[test]\nfn alpha() {}\n#[test]\nfn beta() {}\n",
    );
    write_file(
        repo,
        "flowspec/src/lib.rs",
        "pub fn product() -> bool { true }\n#[cfg(test)]\nmod tests { #[test] fn region() { assert!(true); } }\n",
    );
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(repo)
        .output()
        .expect("scratch Cargo lockfile generator must execute");
    assert_success(&lock, "scratch Cargo.lock generation");
    commit_all(repo, "committed custody inventory fixture");
    temp
}

fn generated_v2_baseline(repo: &Path) -> Value {
    let result = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_success(
        &result,
        "complete scratch baseline generation with committed policy",
    );
    let baseline = read_json(&repo.join("tests/test-custody-baseline.json"));
    assert_eq!(
        baseline.get("schema_version").and_then(Value::as_u64),
        Some(2),
        "schema_version 2 required from clean committed source; generated baseline={baseline}"
    );
    assert_eq!(
        baseline.get("source_sha").and_then(Value::as_str),
        Some(git_text(repo, &["rev-parse", "HEAD"]).as_str()),
        "scratch baseline must bind committed source"
    );
    let verified = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_success(&verified, "unmodified complete v2 baseline must verify");
    baseline
}

fn assert_baseline_mutation(repo: &Path, original: &Value, changed: &Value, reason: &str) {
    assert_ne!(
        original, changed,
        "schema mutation must change exactly one case"
    );
    let baseline_path = repo.join("tests/test-custody-baseline.json");
    fs::write(&baseline_path, serde_json::to_vec_pretty(changed).unwrap()).unwrap();
    let result = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(&result, "isolated complete-v2 mutation", &[reason]);
    assert!(
        !String::from_utf8_lossy(&result.stderr).contains("Traceback"),
        "schema rejection must not traceback: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    fs::write(&baseline_path, serde_json::to_vec_pretty(original).unwrap()).unwrap();
}

fn sorted_unique_strings(value: &Value, contract: &str) -> Vec<String> {
    let rows: Vec<String> = value
        .as_array()
        .unwrap_or_else(|| panic!("{contract} must be an array"))
        .iter()
        .map(|row| {
            row.as_str()
                .unwrap_or_else(|| panic!("{contract} row must be a string"))
                .to_owned()
        })
        .collect();
    assert_eq!(
        rows.iter().cloned().collect::<BTreeSet<_>>().len(),
        rows.len(),
        "{contract} duplicate"
    );
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "{contract} must be sorted"
    );
    rows
}

// -----------------------------------------------------------------------------
// G1 — mixed Rust custody
// -----------------------------------------------------------------------------

fn setup_g1_repo() -> (TempDir, String) {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    let lib_rs = r#"pub fn product_fn() -> u32 {
    42
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_helper() -> u32 {
        10
    }

    #[test]
    fn test_inline_item() {
        assert_eq!(product_fn() + test_helper(), 52);
    }
}

#[cfg(test)]
mod ext_test;
"#;
    write_file(repo, "flowspec/src/lib.rs", lib_rs);
    write_file(
        repo,
        "flowspec/src/ext_test.rs",
        "#[test]\nfn test_external_mod() {\n    assert!(true);\n}\n",
    );
    write_file(repo, "tests/test_dummy.rs", "#[test]\nfn dummy() {}\n");

    let head = commit_all(repo, "G1 initial state");
    (temp, head)
}

#[test]
fn g1_mixed_rust_product_only_edit_passes() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();
    let baseline_path = repo.join("tests/test-custody-baseline.json");

    let gen_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_success(&gen_out, "G1 baseline generation");

    // Product-only edit: change product return value 42 -> 43
    let content = fs::read_to_string(repo.join("flowspec/src/lib.rs")).unwrap();
    let updated = content.replace("42", "43");
    fs::write(repo.join("flowspec/src/lib.rs"), updated).unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_success(&ver_out, "G1 product-only edit must pass test custody");
}

#[test]
fn g1_mixed_rust_inline_assertion_edit_fails() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    // Edit inline assertion 52 -> 53
    let content = fs::read_to_string(repo.join("flowspec/src/lib.rs")).unwrap();
    let updated = content.replace("52", "53");
    fs::write(repo.join("flowspec/src/lib.rs"), updated).unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &ver_out,
        "G1 inline assertion edit must fail custody",
        &["ERROR: region_drift: flowspec/src/lib.rs"],
    );
}

#[test]
fn g1_mixed_rust_adding_test_fails() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    // Add a test function inside mod tests
    let content = fs::read_to_string(repo.join("flowspec/src/lib.rs")).unwrap();
    let updated = content.replace(
        "fn test_inline_item() {",
        "#[test]\n    fn test_new_item() {}\n    fn test_inline_item() {",
    );
    fs::write(repo.join("flowspec/src/lib.rs"), updated).unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &ver_out,
        "G1 adding a test must fail custody",
        &["ERROR: region_drift: flowspec/src/lib.rs"],
    );
}

#[test]
fn g1_mixed_rust_removing_test_fails() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    // Remove the test function
    let content = fs::read_to_string(repo.join("flowspec/src/lib.rs")).unwrap();
    let updated = content.replace(
        "#[test]\n    fn test_inline_item() {\n        assert_eq!(product_fn() + test_helper(), 52);\n    }",
        "",
    );
    fs::write(repo.join("flowspec/src/lib.rs"), updated).unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &ver_out,
        "G1 removing a test must fail custody",
        &["ERROR: region_drift: flowspec/src/lib.rs"],
    );
}

#[test]
fn g1_mixed_rust_changing_helper_inside_test_region_fails() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    // Change helper function return 10 -> 11
    let content = fs::read_to_string(repo.join("flowspec/src/lib.rs")).unwrap();
    let updated = content.replace("10", "11");
    fs::write(repo.join("flowspec/src/lib.rs"), updated).unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &ver_out,
        "G1 changing helper item inside test region must fail custody",
        &["ERROR: region_drift: flowspec/src/lib.rs"],
    );
}

#[test]
fn g1_mixed_rust_changing_external_test_module_fails() {
    let (temp, _head) = setup_g1_repo();
    let repo = temp.path();

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    // Modify external test module
    fs::write(
        repo.join("flowspec/src/ext_test.rs"),
        "#[test]\nfn test_external_mod() {\n    assert!(false);\n}\n",
    )
    .unwrap();

    let ver_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &ver_out,
        "G1 changing resolved external test module must fail custody",
        &["ERROR: external_test_module_drift: flowspec/src/ext_test.rs"],
    );
}

// -----------------------------------------------------------------------------
// G2 — exact Cargo and Rustdoc identities
// -----------------------------------------------------------------------------

#[test]
fn g2_exact_cargo_and_rustdoc_identities_inventory() {
    let policy = read_json(&policy_file_path());
    assert_eq!(
        policy.pointer("/cargo_inventory/target_qualified_identities"),
        Some(&Value::Bool(true))
    );
    let temp = setup_inventory_repo();
    let baseline = generated_v2_baseline(temp.path());
    let executable =
        sorted_unique_strings(&baseline["executable_identities"], "executable identities");
    assert_eq!(
        executable,
        vec![
            "custody-fixture|test|inventory|alpha".to_owned(),
            "custody-fixture|test|inventory|beta".to_owned(),
        ],
        "scratch inventory must use package, target kind, target name and exact executable identity"
    );
    let rustdoc = sorted_unique_strings(&baseline["rustdoc_identities"], "rustdoc identities");
    assert!(
        rustdoc.is_empty(),
        "fixture has no Rustdoc tests; exact empty set must be explicit"
    );
    assert!(
        baseline["rustdoc_inventory"].is_array(),
        "Rustdoc inventory must remain separately represented"
    );
}

#[test]
fn g2_exact_cargo_identities_rejects_removed_and_renamed() {
    let temp = setup_inventory_repo();
    let repo = temp.path();
    let original = generated_v2_baseline(repo);
    let identities =
        sorted_unique_strings(&original["executable_identities"], "executable identities");
    assert!(
        identities.len() >= 2,
        "remove and rename negatives need two real identities"
    );
    let mut removed = original.clone();
    removed["executable_identities"]
        .as_array_mut()
        .unwrap()
        .remove(0);
    assert_baseline_mutation(repo, &original, &removed, "executable_identity_mismatch");
    let mut renamed = original.clone();
    renamed["executable_identities"][0] =
        Value::String("custody-fixture|test|inventory|renamed".to_owned());
    assert_eq!(
        renamed["executable_identities"].as_array().unwrap().len(),
        identities.len()
    );
    assert_baseline_mutation(repo, &original, &renamed, "executable_identity_mismatch");
}

// -----------------------------------------------------------------------------
// G3 — schema-v2 strictness
// -----------------------------------------------------------------------------

fn baseline_fixture() -> (TempDir, Value) {
    let temp = setup_inventory_repo();
    let repo = temp.path();
    assert_eq!(
        fs::read(repo.join("tests/test-custody-policy.json")).unwrap(),
        fs::read(policy_file_path()).unwrap(),
        "scratch baseline must use the exact committed policy"
    );
    let baseline = generated_v2_baseline(repo);
    assert_eq!(
        read_json(&repo.join("tests/test-custody-policy.json"))["deny_unknown_fields"],
        Value::Bool(true)
    );
    (temp, baseline)
}

fn first_whole_file(baseline: &Value) -> String {
    baseline["whole_files"]
        .as_object()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone()
}

fn assert_raw_baseline_mutation(repo: &Path, original: &Value, text: &str, reason: &str) {
    let path = repo.join("tests/test-custody-baseline.json");
    fs::write(&path, text).unwrap();
    let result = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(&result, "raw duplicate-key baseline mutation", &[reason]);
    assert!(!String::from_utf8_lossy(&result.stderr).contains("Traceback"));
    fs::write(path, serde_json::to_vec_pretty(original).unwrap()).unwrap();
}

#[test]
fn g3_schema_v2_unknown_top_level_and_nested_fields() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    let mut top = original.clone();
    top["unknown_top_level_field"] = Value::Bool(true);
    assert_baseline_mutation(repo, &original, &top, "schema_unknown_field");
    let mut nested = original.clone();
    nested["cargo_inventory"]["unknown_nested_field"] = Value::Bool(true);
    assert_baseline_mutation(repo, &original, &nested, "schema_unknown_field");
}

#[test]
fn g3_schema_v2_wrong_scalar_object_and_array_types() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    for (field, bad) in [
        ("source_sha", Value::from(12345)),
        ("whole_files", Value::String("not an object".to_owned())),
        (
            "executable_identities",
            Value::String("not an array".to_owned()),
        ),
    ] {
        let mut changed = original.clone();
        changed[field] = bad;
        assert_baseline_mutation(repo, &original, &changed, "schema_invalid_type");
    }
    let mut missing = original.clone();
    missing.as_object_mut().unwrap().remove("source_sha");
    assert_baseline_mutation(repo, &original, &missing, "baseline_missing_required_key");
}

#[test]
fn g3_schema_v2_empty_inventory_maps_rejected() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    assert!(!original["whole_files"].as_object().unwrap().is_empty());
    assert!(!original["rust_regions"].as_array().unwrap().is_empty());
    assert!(!original["executable_identities"]
        .as_array()
        .unwrap()
        .is_empty());
    for (field, empty) in [
        ("whole_files", serde_json::json!({})),
        ("rust_regions", serde_json::json!([])),
        ("executable_identities", serde_json::json!([])),
    ] {
        let mut changed = original.clone();
        changed[field] = empty;
        assert_baseline_mutation(repo, &original, &changed, "schema_empty_universe");
    }
}

#[test]
fn g3_schema_v2_invalid_sha256_length_case_and_alphabet() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    let path = first_whole_file(&original);
    for bad in ["12345".to_owned(), "A".repeat(64), "g".repeat(64)] {
        let mut changed = original.clone();
        changed["whole_files"][&path] = Value::String(bad);
        assert_baseline_mutation(repo, &original, &changed, "schema_invalid_sha256");
    }
}

#[test]
fn g3_schema_v2_duplicate_paths_regions_and_executable_identities() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    let path = first_whole_file(&original);
    let digest = original["whole_files"][&path].as_str().unwrap();
    let baseline_text = serde_json::to_string(&original).unwrap();
    let key = format!("\"{path}\":\"{digest}\"");
    assert_eq!(baseline_text.matches(&key).count(), 1);
    let duplicate_key = format!("{key},{key}");
    assert_raw_baseline_mutation(
        repo,
        &original,
        &baseline_text.replacen(&key, &duplicate_key, 1),
        "schema_duplicate_path",
    );
    let regions = original["rust_regions"].as_array().unwrap();
    assert!(!regions.is_empty());
    let mut duplicate_region = original.clone();
    duplicate_region["rust_regions"]
        .as_array_mut()
        .unwrap()
        .push(regions[0].clone());
    assert_baseline_mutation(
        repo,
        &original,
        &duplicate_region,
        "schema_duplicate_region",
    );
    let identities = original["executable_identities"].as_array().unwrap();
    assert!(!identities.is_empty());
    let mut duplicate_identity = original.clone();
    duplicate_identity["executable_identities"]
        .as_array_mut()
        .unwrap()
        .push(identities[0].clone());
    assert_baseline_mutation(
        repo,
        &original,
        &duplicate_identity,
        "schema_duplicate_identity",
    );
}

#[test]
fn g3_schema_v2_phantom_missing_and_noncanonical_paths() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    let path = first_whole_file(&original);
    let digest = original["whole_files"][&path].clone();
    let mut phantom = original.clone();
    phantom["whole_files"]["tests/nonexistent_phantom.rs"] = digest.clone();
    assert_baseline_mutation(repo, &original, &phantom, "baseline_phantom_path");
    let mut missing = original.clone();
    missing["whole_files"]
        .as_object_mut()
        .unwrap()
        .remove(&path);
    assert_baseline_mutation(repo, &original, &missing, "baseline_missing_path");
    let mut noncanonical = original.clone();
    noncanonical["whole_files"]
        .as_object_mut()
        .unwrap()
        .remove(&path);
    noncanonical["whole_files"][format!("tests/../{path}")] = digest;
    assert_baseline_mutation(repo, &original, &noncanonical, "schema_noncanonical_path");
}

#[test]
fn g3_schema_v2_malformed_legacy_policy_platform_command_bindings() {
    let (temp, original) = baseline_fixture();
    let repo = temp.path();
    let mut cases: Vec<(Value, &str)> = Vec::new();
    let mut legacy = original.clone();
    legacy["legacy"]["candidate_sha"] = Value::String("f".repeat(40));
    cases.push((legacy, "schema_invalid_binding"));
    let mut policy = original.clone();
    policy["policy_sha256"] = Value::String("0".repeat(64));
    cases.push((policy, "policy_hash_mismatch"));
    let mut platform = original.clone();
    platform["cargo_inventory"]["platform_contract"]["os"] = Value::String("windows".to_owned());
    cases.push((platform, "schema_invalid_binding"));
    let mut command = original.clone();
    command["cargo_inventory"]["command"] =
        Value::String("cargo test --workspace --all-features -- --ignored".to_owned());
    cases.push((command, "schema_invalid_binding"));
    let mut count = original.clone();
    count["counts"]["whole_files"] =
        Value::from(original["counts"]["whole_files"].as_u64().unwrap() + 1);
    cases.push((count, "schema_count_mismatch"));
    let path = first_whole_file(&original);
    let mut digest = original.clone();
    digest["whole_files"][&path] = Value::String("0".repeat(64));
    cases.push((digest, "baseline_hash_mismatch"));
    let mut schema = original.clone();
    schema["schema_version"] = Value::from(1);
    cases.push((schema, "schema_invalid_binding"));
    for (changed, reason) in cases {
        assert_baseline_mutation(repo, &original, &changed, reason);
    }
}

// -----------------------------------------------------------------------------
// G4 — filesystem loss and race closure
// -----------------------------------------------------------------------------

#[cfg(unix)]
fn primary_path_reason_matches(
    stderr: &str,
    expected_path: &str,
    expected_position: usize,
    expected_reason: &str,
) -> bool {
    stderr.lines().any(|line| {
        let mut fields = line.splitn(3, ':');
        let (Some(path), Some(position), Some(message)) =
            (fields.next(), fields.next(), fields.next())
        else {
            return false;
        };
        path == expected_path
            && position.parse::<usize>().ok() == Some(expected_position)
            && message.split_whitespace().next() == Some(expected_reason)
    })
}

#[cfg(unix)]
fn assert_primary_path_reason(
    output: &Output,
    expected_path: &str,
    expected_position: usize,
    expected_reason: &str,
) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        primary_path_reason_matches(&stderr, expected_path, expected_position, expected_reason),
        "missing primary {expected_path}:{expected_position}: {expected_reason}; stderr={stderr}"
    );
}

#[cfg(unix)]
fn run_python_probe(repo: &Path, script: &str, target: &Path, timeout: Duration) -> Output {
    let mut child = Command::new("python3")
        .arg("-c")
        .arg(script)
        .arg(checker_path())
        .arg(repo)
        .arg(target)
        .current_dir(repo)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("bounded checker probe must spawn");
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .expect("probe status must be observable")
            .is_some()
        {
            return child
                .wait_with_output()
                .expect("probe output must be collected");
        }
        if start.elapsed() >= timeout {
            child.kill().expect("timed-out probe must be killed");
            child.wait().expect("timed-out probe must be reaped");
            panic!(
                "bounded checker probe timed out after {} ms",
                timeout.as_millis()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn g4_unix_domain_socket_and_race_closure() {
    #[cfg(unix)]
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(repo, "tests/socket.rs", "// indexed regular source\n");
        commit_all(repo, "tracked regular socket target");
        let socket_path = repo.join("tests/socket.rs");
        fs::remove_file(&socket_path).unwrap();
        let _listener =
            UnixListener::bind(&socket_path).expect("Unix socket must bind at tracked path");
        git(repo, &["ls-files", "--error-unmatch", "tests/socket.rs"]);
        assert!(
            fs::symlink_metadata(&socket_path)
                .unwrap()
                .file_type()
                .is_socket(),
            "the indexed target must now be a live Unix socket"
        );
        let out = run_checker_with_timeout(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--check-markers",
            ],
            Duration::from_secs(5),
        )
        .expect("bounded socket checker must return");
        assert_failure_evidence(
            &out,
            "G4 tracked Unix socket must reach source_non_regular classifier",
            &["tests/socket.rs:0: source_non_regular"],
        );
        assert_primary_path_reason(&out, "tests/socket.rs", 0, "source_non_regular");
        assert!(
            !primary_path_reason_matches(
                "tests/socket.rs:0: source_not_regular source_non_regular",
                "tests/socket.rs",
                0,
                "source_non_regular"
            ),
            "a trailing alias must not satisfy the primary reason contract"
        );
    }
}

#[test]
fn g4_changed_during_read_race_witness() {
    #[cfg(unix)]
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(repo, "tests/race_target.rs", "// original content\n");
        commit_all(repo, "tracked race target");
        write_file(repo, "replacement.staged", "// changed! content\n");
        assert_eq!(
            fs::metadata(repo.join("tests/race_target.rs"))
                .unwrap()
                .len(),
            fs::metadata(repo.join("replacement.staged")).unwrap().len(),
            "race replacement must preserve byte length"
        );
        let script = r#"
import importlib.util, os, sys
checker, root, target = sys.argv[1:]
spec = importlib.util.spec_from_file_location('custody_probe', checker)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
original_read = os.read
reached = False

def read_and_replace(fd, count):
    global reached
    data = original_read(fd, count)
    if not reached and os.path.realpath(f'/proc/self/fd/{fd}') == target:
        os.replace(os.path.join(root, 'replacement.staged'), target)
        reached = True
    return data

os.read = read_and_replace
sys.argv = [checker, '--root', root, '--policy', 'tests/test-custody-policy.json', '--check-markers']
result = module.main()
if reached:
    print('WITNESS_RACE_REPLACED_AFTER_OPEN', file=sys.stderr)
sys.exit(result)
"#;
        let out = run_python_probe(
            repo,
            script,
            &repo.join("tests/race_target.rs"),
            Duration::from_secs(5),
        );
        assert_failure_evidence(
            &out,
            "G4 synchronized source_changed_during_read must be observed by real checker main",
            &[
                "tests/race_target.rs:0: source_changed_during_read",
                "WITNESS_RACE_REPLACED_AFTER_OPEN",
            ],
        );
        assert_primary_path_reason(
            &out,
            "tests/race_target.rs",
            0,
            "source_changed_during_read",
        );
    }
}

#[test]
fn g4_post_open_regular_file_classifier_rejects_non_regular() {
    #[cfg(unix)]
    {
        let device = Path::new("/dev/null");
        assert!(device.exists(), "real character device witness must exist");
        assert!(fs::metadata(device).unwrap().file_type().is_char_device());
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(repo, "tests/dev_null.rs", "// indexed regular source\n");
        commit_all(repo, "tracked descriptor target");
        let script = r#"
import errno, importlib.util, os, stat, sys
checker, root, target = sys.argv[1:]
spec = importlib.util.spec_from_file_location('custody_probe', checker)
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
original_open = os.open
opened = []

def open_device(path, flags, *args, **kwargs):
    if os.fspath(path) == target:
        fd = original_open('/dev/null', os.O_RDONLY)
        assert stat.S_ISCHR(os.fstat(fd).st_mode)
        opened.append(fd)
        return fd
    return original_open(path, flags, *args, **kwargs)

os.open = open_device
sys.argv = [checker, '--root', root, '--policy', 'tests/test-custody-policy.json', '--check-markers']
result = module.main()
if opened:
    assert all(is_closed(fd) for fd in opened)
    print('WITNESS_CHAR_DEVICE_OPENED_AND_FD_CLOSED', file=sys.stderr)
sys.exit(result)
"#.replace("assert all(is_closed(fd) for fd in opened)", "\n    for fd in opened:\n        try:\n            os.fstat(fd)\n        except OSError as error:\n            assert error.errno == errno.EBADF\n        else:\n            raise AssertionError('descriptor leaked')\n");
        let out = run_python_probe(
            repo,
            &script,
            &repo.join("tests/dev_null.rs"),
            Duration::from_secs(5),
        );
        assert_failure_evidence(
            &out,
            "G4 real post-open nonregular descriptor must be rejected and closed",
            &[
                "tests/dev_null.rs:0: source_non_regular",
                "WITNESS_CHAR_DEVICE_OPENED_AND_FD_CLOSED",
            ],
        );
        assert_primary_path_reason(&out, "tests/dev_null.rs", 0, "source_non_regular");

        let link = repo.join("tests/dev_null.rs");
        fs::remove_file(&link).unwrap();
        symlink(device, &link).expect("tracked final-component symlink must be created");
        git(repo, &["ls-files", "--error-unmatch", "tests/dev_null.rs"]);
        let symlink_out = run_checker(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--check-markers",
            ],
        );
        assert_failure_evidence(
            &symlink_out,
            "G4 no-follow final symlink is a separate branch",
            &["tests/dev_null.rs:0: source_final_symlink"],
        );
        assert_primary_path_reason(&symlink_out, "tests/dev_null.rs", 0, "source_final_symlink");
    }
}

// -----------------------------------------------------------------------------
// G5 — atomic generation on every loss class
// -----------------------------------------------------------------------------

#[test]
fn g5_atomic_generation_refuses_all_loss_classes_and_retains_sentinel() {
    let sentinel = b"SENTINEL_BYTES_123456789_MUST_NOT_BE_OVERWRITTEN\n";

    // Loss Class 1: Dirty repository tree
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(repo, "tests/test_clean.rs", "#[test]\nfn test_c() {}\n");
        commit_all(repo, "clean");

        // Make dirty
        write_file(
            repo,
            "tests/test_clean.rs",
            "#[test]\nfn test_c() { // dirty }\n",
        );

        let baseline_path = repo.join("tests/test-custody-baseline.json");
        fs::write(&baseline_path, sentinel).unwrap();

        let out = run_checker(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--generate-baseline",
                "tests/test-custody-baseline.json",
            ],
        );
        assert_failure_evidence(
            &out,
            "G5 baseline generation must refuse dirty source tree",
            &["dirty_source_tree"],
        );
        let actual = fs::read(&baseline_path).unwrap();
        assert_eq!(
            actual, sentinel,
            "G5 baseline file must retain exact sentinel bytes on dirty tree refusal"
        );
    }

    // Loss Class 2: Missing required whole-root path
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(repo, "tests/missing_target.rs", "#[test]\nfn t() {}\n");
        commit_all(repo, "committed");
        fs::remove_file(repo.join("tests/missing_target.rs")).unwrap();

        let baseline_path = repo.join("tests/test-custody-baseline.json");
        fs::write(&baseline_path, sentinel).unwrap();

        let out = run_checker(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--generate-baseline",
                "tests/test-custody-baseline.json",
            ],
        );
        assert!(
            !out.status.success(),
            "G5 baseline generation must refuse when whole-root file is lost"
        );
        let actual = fs::read(&baseline_path).unwrap();
        assert_eq!(
            actual, sentinel,
            "G5 baseline file must retain exact sentinel bytes on missing path loss"
        );
    }

    // Loss Class 3: Read loss (mode 000 file)
    #[cfg(unix)]
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        let unreadable = repo.join("tests/unreadable.rs");
        fs::write(&unreadable, "#[test]\nfn t() {}\n").unwrap();
        commit_all(repo, "committed");
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let baseline_path = repo.join("tests/test-custody-baseline.json");
        fs::write(&baseline_path, sentinel).unwrap();

        let out = run_checker(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--generate-baseline",
                "tests/test-custody-baseline.json",
            ],
        );
        assert!(
            !out.status.success(),
            "G5 baseline generation must refuse on unreadable file loss"
        );
        let actual = fs::read(&baseline_path).unwrap();
        assert_eq!(
            actual, sentinel,
            "G5 baseline file must retain exact sentinel bytes on read loss"
        );
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o644)).unwrap();
    }

    // Loss Class 4: Lexical parse loss (invalid UTF-8)
    {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);
        write_file(
            repo,
            "tests/bad_utf8.rs",
            b"fn test_bad() { \xff\xfe\xfd }\n",
        );
        commit_all(repo, "committed bad utf8");

        let baseline_path = repo.join("tests/test-custody-baseline.json");
        fs::write(&baseline_path, sentinel).unwrap();

        let out = run_checker(
            repo,
            &[
                "--policy",
                "tests/test-custody-policy.json",
                "--generate-baseline",
                "tests/test-custody-baseline.json",
            ],
        );
        assert!(
            !out.status.success(),
            "G5 baseline generation must refuse on invalid UTF-8 lexical parse loss"
        );
        let actual = fs::read(&baseline_path).unwrap();
        assert_eq!(
            actual, sentinel,
            "G5 baseline file must retain exact sentinel bytes on parse loss"
        );
    }
}

// -----------------------------------------------------------------------------
// G6 — lexical negative controls
// -----------------------------------------------------------------------------

#[test]
fn g6_rust_lexical_negative_controls_and_nested_comment_quotes() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    let m_ignore = marker(&["#", "[", "ignore", "]"]);
    let mut rust_content = String::from("// Negative controls for Rust lexing\n");
    rust_content.push_str("pub fn rust_controls() {\n");
    rust_content.push_str(&format!("    let s = \"{m_ignore}\";\n"));
    rust_content.push_str(&format!("    let b = b\"{m_ignore}\";\n"));
    rust_content.push_str(&format!("    let r = r#\"{m_ignore}\"#;\n"));
    rust_content.push_str(&format!("    let br = br#\"{m_ignore}\"#;\n"));
    rust_content.push_str("    let c = 'i';\n");
    rust_content.push_str(&format!("    let line_com = (); // {m_ignore}\n"));
    rust_content.push_str(&format!("    let block_com = (); /* {m_ignore} */\n"));
    rust_content.push_str(&format!(
        "    let nested_com = (); /* \"quoted text inside block comment\" {m_ignore} */\n"
    ));
    rust_content.push_str("}\n");
    write_file(repo, "tests/rust_controls.rs", rust_content);

    let m_spaced = marker(&["#", " ", "[", "ignore", "]"]);
    let rust_spaced = format!("{m_spaced}\nfn spaced() {{}}\n");
    write_file(repo, "tests/rust_spaced.rs", rust_spaced);
    commit_all(repo, "Rust controls and spaced ignore");

    let out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--check-markers",
        ],
    );
    assert_failure_evidence(
        &out,
        "G6 Rust spaced ignore # [ignore] must be detected as live marker",
        &["tests/rust_spaced.rs:1: rust_ignore"],
    );
}

#[test]
fn g6_javascript_lexical_controls_and_template_interpolation() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    let m_skip = marker(&["test", ".", "skip"]);

    // Negative controls: string, comment, regex, template text outside interpolation
    let js_clean = format!(
        r#"// JS negative controls
const s1 = '{m_skip}';
const s2 = "{m_skip}";
// {m_skip}
/* {m_skip} */
const rx = /{m_skip}/;
const t1 = `{m_skip}`;
"#
    );
    write_file(repo, "tests/js_clean.js", js_clean);

    // Executable interpolation: marker inside ${...} MUST be detected
    let js_live = format!("const live = `${{{m_skip}('x', () => {{}})}}`;\n");
    write_file(repo, "tests/js_live.js", js_live);
    commit_all(repo, "JS controls and live interpolation");

    let out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--check-markers",
        ],
    );
    assert_failure_evidence(
        &out,
        "G6 JS marker inside ${...} template interpolation must remain live and be reported",
        &["tests/js_live.js:1: javascript_test_skip"],
    );
}

#[test]
fn g6_python_lexical_controls_and_executable_fstrings() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    let m_pyskip = marker(&["pytest", ".", "skip"]);

    // Negative controls: comment, normal/raw/byte/triple strings
    let py_clean = format!(
        r#"# Python negative controls
# {m_pyskip}()
s1 = "{m_pyskip}"
s2 = r"{m_pyskip}"
b1 = b"{m_pyskip}"
rb1 = rb"{m_pyskip}"
fr1 = fr"{m_pyskip}"
t1 = '''{m_pyskip}'''
t2 = """{m_pyskip}"""
"#
    );
    write_file(repo, "tests/py_clean.py", py_clean);

    // Executable f-string: marker inside {...} MUST be detected
    let py_live = format!("f_live = f\"{{{m_pyskip}()}}\"\n");
    write_file(repo, "tests/py_live.py", py_live);
    commit_all(repo, "Python controls and live f-strings");

    let out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--check-markers",
        ],
    );
    assert_failure_evidence(
        &out,
        "G6 Python marker inside f-string expression {...} must remain live and be reported",
        &["tests/py_live.py:1: python_test_skip"],
    );
}

// -----------------------------------------------------------------------------
// G7 — policy authority and scratch realism
// -----------------------------------------------------------------------------

#[test]
fn g7_policy_authority_hash_mismatch_unknown_field_and_ignored_policy() {
    let temp = setup_inventory_repo();
    let repo = temp.path();
    let original = generated_v2_baseline(repo);

    let mut forged = original.clone();
    forged["policy_sha256"] = Value::String("0".repeat(64));
    assert_baseline_mutation(repo, &original, &forged, "policy_hash_mismatch");

    // Case 2: Unknown policy field in policy file
    let mut policy: Value = read_json(&policy_file_path());
    policy["unknown_policy_setting"] = Value::Bool(true);
    write_file(
        repo,
        "tests/bad_policy.json",
        serde_json::to_string_pretty(&policy).unwrap(),
    );

    let out2 = run_checker(
        repo,
        &[
            "--policy",
            "tests/bad_policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &out2,
        "G7 unknown field in policy file must fail closed under deny_unknown_fields",
        &["policy_unknown_field"],
    );
    assert!(!String::from_utf8_lossy(&out2.stderr).contains("Traceback"));

    // Case 3: Ignored policy invocation (running baseline verification without passing --policy)
    let out3 = run_checker(repo, &["--baseline", "tests/test-custody-baseline.json"]);
    assert_failure_evidence(
        &out3,
        "G7 verification without passing --policy must fail closed",
        &["policy_required"],
    );
    assert!(!String::from_utf8_lossy(&out3.stderr).contains("Traceback"));
}
