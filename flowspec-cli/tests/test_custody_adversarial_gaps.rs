use serde_json::Value;
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
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
    let inventory_cfg = policy.get("cargo_inventory").expect("cargo inventory");
    assert_eq!(
        inventory_cfg
            .get("target_qualified_identities")
            .and_then(Value::as_bool),
        Some(true),
        "cargo inventory policy must specify target-qualified identities"
    );

    // Verify current repo baseline or generated inventory binding
    let baseline_path = repo_root().join("tests/test-custody-baseline.json");
    if baseline_path.exists() {
        let baseline = read_json(&baseline_path);
        if let Some(identities) = baseline
            .get("executable_identities")
            .and_then(Value::as_array)
        {
            assert!(
                identities.len() >= 2369,
                "executable identities count must be >= 2369 starting set, got {}",
                identities.len()
            );
        }
        assert!(
            baseline.get("rustdoc_identities").is_some(),
            "rustdoc_identities must be recorded separately and exactly (zero is valid set, never omitted)"
        );
    }
}

#[test]
fn g2_exact_cargo_identities_rejects_removed_and_renamed() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(
        repo,
        "tests/test_main.rs",
        "#[test]\nfn test_alpha() {}\n#[test]\nfn test_beta() {}\n",
    );
    commit_all(repo, "G2 setup");

    let gen_out = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_success(&gen_out, "G2 baseline generation");

    let baseline_file = repo.join("tests/test-custody-baseline.json");
    if baseline_file.exists() {
        let mut baseline: Value =
            serde_json::from_str(&fs::read_to_string(&baseline_file).unwrap()).unwrap();

        // Case 1: Renamed identity (same count)
        if let Some(arr) = baseline
            .get_mut("executable_identities")
            .and_then(Value::as_array_mut)
        {
            if !arr.is_empty() {
                arr[0] = Value::String("test_renamed_identity".into());
            }
        }
        fs::write(
            &baseline_file,
            serde_json::to_string_pretty(&baseline).unwrap(),
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
            "G2 renamed identity must be rejected even when total count is unchanged",
            &["ERROR: executable_identity_mismatch"],
        );
    } else {
        // Against old checker, verify baseline fails with missing evidence
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
            "G2 exact identity set verification",
            &["ERROR: executable_identity_mismatch"],
        );
    }
}

// -----------------------------------------------------------------------------
// G3 — schema-v2 strictness
// -----------------------------------------------------------------------------

#[test]
fn g3_schema_v2_unknown_top_level_and_nested_fields() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let mut baseline: Value = if baseline_path.exists() {
        serde_json::from_str(&fs::read_to_string(&baseline_path).unwrap())
            .unwrap_or(serde_json::json!({}))
    } else {
        serde_json::json!({"schema_version": 2})
    };

    // Unknown top-level field
    baseline["unknown_top_level_field"] = Value::Bool(true);
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 unknown top-level field must be rejected under deny_unknown_fields",
        &["schema_unknown_field"],
    );
}

#[test]
fn g3_schema_v2_wrong_scalar_object_and_array_types() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--generate-baseline",
            "tests/test-custody-baseline.json",
        ],
    );

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let mut baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": 12345, // wrong scalar type (integer instead of string)
        "whole_files": "not_an_object", // wrong object type
        "executable_identities": "not_an_array" // wrong array type
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 wrong scalar, object, and array types must be rejected deterministically",
        &["schema_invalid_type"],
    );
}

#[test]
fn g3_schema_v2_empty_inventory_maps_rejected() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": git_text(repo, &["rev-parse", "HEAD"]),
        "whole_files": {}, // empty whole-file map when discovered universe is non-empty
        "rust_regions": [],
        "executable_identities": []
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 empty whole-file map when universe non-empty must be rejected",
        &["schema_empty_universe"],
    );
}

#[test]
fn g3_schema_v2_invalid_sha256_length_case_and_alphabet() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": git_text(repo, &["rev-parse", "HEAD"]),
        "whole_files": {
            "tests/test_one.rs": "12345" // invalid sha256 length
        }
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 invalid sha256 length/case/alphabet must be rejected",
        &["schema_invalid_sha256"],
    );
}

#[test]
fn g3_schema_v2_duplicate_paths_regions_and_executable_identities() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline_str = r#"{
  "schema": "flowspec.test_custody_baseline",
  "schema_version": 2,
  "source_sha": "a102deb681f0180985b4fdfb9cc544ed7d8db90e",
  "executable_identities": [
    "test_one",
    "test_one"
  ]
}"#;
    fs::write(&baseline_path, baseline_str).unwrap();

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
        "G3 duplicate executable identities must be rejected",
        &["schema_duplicate_identity"],
    );
}

#[test]
fn g3_schema_v2_phantom_missing_and_noncanonical_paths() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": git_text(repo, &["rev-parse", "HEAD"]),
        "whole_files": {
            "tests/nonexistent_phantom.rs": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        }
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 phantom file entry must be rejected",
        &["baseline_phantom_path"],
    );
}

#[test]
fn g3_schema_v2_malformed_legacy_policy_platform_command_bindings() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_one.rs", "#[test]\nfn test_one() {}\n");
    commit_all(repo, "G3 setup");

    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": git_text(repo, &["rev-parse", "HEAD"]),
        "cargo_inventory": {
            "platform_contract": {
                "os": "windows" // wrong platform
            }
        }
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
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
        "G3 malformed platform/command binding must be rejected",
        &["schema_invalid_binding"],
    );
}

// -----------------------------------------------------------------------------
// G4 — filesystem loss and race closure
// -----------------------------------------------------------------------------

#[test]
fn g4_unix_domain_socket_and_race_closure() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    #[cfg(unix)]
    {
        let socket_path = repo.join("tests/socket.rs");
        let _listener = UnixListener::bind(&socket_path).expect("unix domain socket must bind");
        git(repo, &["add", "tests/socket.rs"]);

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
            "G4 Unix domain socket must fail closed with deterministic reason",
            &["tests/socket.rs:0: source_non_regular"],
        );
    }
}

#[test]
fn g4_changed_during_read_race_witness() {
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);

    let race_file = repo.join("tests/race_target.rs");
    fs::write(&race_file, "// initial\n").unwrap();
    git(repo, &["add", "tests/race_target.rs"]);

    let running = Arc::new(AtomicBool::new(true));
    let r_clone = running.clone();
    let f_path = race_file.clone();

    let writer = thread::spawn(move || {
        let mut toggle = false;
        while r_clone.load(Ordering::Relaxed) {
            let data = if toggle {
                "// small content\n"
            } else {
                "// much longer content line to change size during read operation\n"
            };
            toggle = !toggle;
            let _ = fs::write(&f_path, data);
            thread::sleep(Duration::from_micros(100));
        }
    });

    let result = run_checker_with_timeout(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--check-markers",
        ],
        Duration::from_secs(3),
    );
    running.store(false, Ordering::Relaxed);
    let _ = writer.join();

    match result {
        Ok(out) => {
            assert_failure_evidence(
                &out,
                "G4 changed-during-read file must be observed and reported",
                &["tests/race_target.rs:0: source_changed_during_read"],
            );
        }
        Err(e) => {
            panic!("G4 race witness execution error or timeout: {e}");
        }
    }
}

#[test]
fn g4_post_open_regular_file_classifier_rejects_non_regular() {
    // Test post-open regular file classifier on available non-regular descriptors
    let dev_null = Path::new("/dev/null");
    if dev_null.exists() {
        let temp = init_repo();
        let repo = temp.path();
        copy_policy_and_harness(repo);

        #[cfg(unix)]
        {
            let link_target = repo.join("tests/dev_null.rs");
            let _ = symlink(dev_null, &link_target);
            git(repo, &["add", "tests/dev_null.rs"]);

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
                "G4 non-regular descriptor must be rejected by shared classifier",
                &["tests/dev_null.rs:0: source_non_regular"],
            );
        }
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
    let temp = init_repo();
    let repo = temp.path();
    copy_policy_and_harness(repo);
    write_file(repo, "tests/test_p.rs", "#[test]\nfn tp() {}\n");
    commit_all(repo, "G7 setup");

    // Case 1: Policy hash mismatch in baseline
    let baseline_path = repo.join("tests/test-custody-baseline.json");
    let baseline: Value = serde_json::json!({
        "schema": "flowspec.test_custody_baseline",
        "schema_version": 2,
        "source_sha": git_text(repo, &["rev-parse", "HEAD"]),
        "policy_sha256": "0000000000000000000000000000000000000000000000000000000000000000" // hash mismatch
    });
    fs::write(
        &baseline_path,
        serde_json::to_string_pretty(&baseline).unwrap(),
    )
    .unwrap();

    let out1 = run_checker(
        repo,
        &[
            "--policy",
            "tests/test-custody-policy.json",
            "--baseline",
            "tests/test-custody-baseline.json",
        ],
    );
    assert_failure_evidence(
        &out1,
        "G7 baseline policy hash mismatch must fail closed",
        &["policy_hash_mismatch"],
    );

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

    // Case 3: Ignored policy invocation (running baseline verification without passing --policy)
    let out3 = run_checker(repo, &["--baseline", "tests/test-custody-baseline.json"]);
    assert_failure_evidence(
        &out3,
        "G7 verification without passing --policy must fail closed",
        &["policy_required"],
    );
}
