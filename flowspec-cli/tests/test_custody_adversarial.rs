use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
#[cfg(unix)]
use std::os::unix::ffi::OsStringExt;
#[cfg(unix)]
use std::os::unix::fs::{symlink, PermissionsExt};
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

#[cfg(unix)]
fn git_add_path_bytes(repo: &Path, path: &[u8]) {
    let mut child = Command::new("git")
        .args(["add", "--pathspec-from-file=-", "--pathspec-file-nul"])
        .current_dir(repo)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git add with pathspec stdin must spawn");
    child
        .stdin
        .as_mut()
        .expect("git add stdin must be open")
        .write_all(path)
        .expect("pathspec bytes must be written");
    child
        .stdin
        .as_mut()
        .expect("git add stdin must be open")
        .write_all(&[0])
        .expect("pathspec NUL must be written");
    let output = child
        .wait_with_output()
        .expect("git add with pathspec stdin must finish");
    assert!(
        output.status.success(),
        "git add invalid path failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
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
    let mut command = Command::new("python");
    command.arg(checker_path()).arg("--root").arg(repo);
    if !args.contains(&"--policy") {
        command.arg("--policy").arg(policy_file_path());
    }
    command
        .args(args)
        .output()
        .expect("custody checker must execute")
}

fn run_checker_with_timeout(
    repo: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    let mut command = Command::new("python");
    command.arg(checker_path()).arg("--root").arg(repo);
    if !args.contains(&"--policy") {
        command.arg("--policy").arg(policy_file_path());
    }
    let mut child = command
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

fn generate_baseline(repo: &Path, baseline: &Path) -> Output {
    run_checker(
        repo,
        &[
            "--generate-baseline",
            baseline.to_str().expect("baseline path must be UTF-8"),
        ],
    )
}

fn verify_baseline(repo: &Path, baseline: &Path) -> Output {
    run_checker(
        repo,
        &[
            "--baseline",
            baseline.to_str().expect("baseline path must be UTF-8"),
        ],
    )
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).expect("JSON file must be readable"))
        .expect("JSON must parse")
}

fn marker(parts: &[&str]) -> String {
    parts.concat()
}

#[test]
fn policy_file_declares_schema_v1_custody_authority() {
    let policy_path = repo_root().join("tests/test-custody-policy.json");
    let policy = read_json(&policy_path);

    assert_eq!(policy["schema"], "flowspec.test_custody_policy");
    assert_eq!(policy["schema_version"], 1);
    assert_eq!(policy["deny_unknown_fields"], true);
    assert_eq!(
        policy["control_baseline"]["path"],
        "tests/test-custody-baseline.json"
    );

    let extensions: BTreeSet<&str> = policy["marker_scan"]["extensions"]
        .as_array()
        .expect("extensions must be an array")
        .iter()
        .map(|value| value.as_str().expect("extension must be string"))
        .collect();
    assert_eq!(
        extensions,
        BTreeSet::from([".rs", ".js", ".ts", ".tsx", ".jsx", ".py"])
    );
}

#[test]
fn marker_scan_requires_specs_for_all_six_extensions() {
    let temp = init_repo();
    let repo = temp.path();
    let rust_ignore = marker(&["#", "[", "ignore", "]"]);
    let js_skip = marker(&["test", ".", "skip"]);
    let ts_skip = marker(&["it", ".", "skip"]);
    let jsx_skip = marker(&["describe", ".", "skip"]);
    let tsx_skip = marker(&["suite", ".", "skip"]);
    let py_skip = marker(&["@", "pytest", ".", "mark", ".", "skip"]);

    write_file(
        repo,
        "tests/live_rust.rs",
        format!("{rust_ignore}\nfn case() {{}}\n"),
    );
    write_file(
        repo,
        "tests/live_js.js",
        format!("{js_skip}('x', () => {{}});\n"),
    );
    write_file(
        repo,
        "tests/live_ts.ts",
        format!("{ts_skip}('x', () => {{}});\n"),
    );
    write_file(
        repo,
        "tests/live_jsx.jsx",
        format!("{jsx_skip}('x', () => <div />);\n"),
    );
    write_file(
        repo,
        "tests/live_tsx.tsx",
        format!("{tsx_skip}('x', () => <div />);\n"),
    );
    write_file(
        repo,
        "tests/live_py.py",
        format!("{py_skip}\ndef case():\n    pass\n"),
    );
    commit_all(repo, "add six-extension marker probes");

    let output = run_checker(repo, &["--check-markers"]);
    assert_failure_evidence(
        &output,
        "marker scan must cover every supported extension",
        &[
            "tests/live_rust.rs:1: rust_ignore",
            "tests/live_js.js:1: javascript_test_skip",
            "tests/live_ts.ts:1: typescript_test_skip",
            "tests/live_jsx.jsx:1: jsx_suite_skip",
            "tests/live_tsx.tsx:1: tsx_suite_skip",
            "tests/live_py.py:1: python_test_skip",
        ],
    );
}

#[test]
fn rust_ignore_forms_and_lexical_controls_are_classified() {
    let temp = init_repo();
    let repo = temp.path();
    let spaced = marker(&["#", " ", "[", "ignore", "]"]);
    let cfg_attr = marker(&["#", "[", "cfg_attr(any(), ", "ignore", ")", "]"]);
    let ignored_in_raw_byte = marker(&["#", "[", "ignore", "]"]);
    let lifetime = marker(&["'", "ignore"]);

    write_file(
        repo,
        "tests/spaced_ignore.rs",
        format!("#[test]\n{spaced}\nfn spaced() {{}}\n"),
    );
    write_file(
        repo,
        "tests/conditional_ignore.rs",
        format!("{cfg_attr}\n#[test]\nfn conditional() {{}}\n"),
    );
    write_file(
        repo,
        "tests/raw_byte_and_lifetime_controls.rs",
        format!(
            "#[test]\nfn controls() {{\n    let _bytes = br#\"{ignored_in_raw_byte}\"#;\n    let _label = {lifetime};\n    // {ignored_in_raw_byte}\n}}\n"
        ),
    );
    commit_all(repo, "add Rust ignore adversaries");

    let output = run_checker(repo, &["--check-markers"]);
    assert_failure_evidence(
        &output,
        "Rust spaced and conditional ignore forms must be rejected while controls stay clean",
        &[
            "tests/spaced_ignore.rs:2: rust_ignore_spaced",
            "tests/conditional_ignore.rs:1: rust_ignore_cfg_attr",
        ],
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("raw_byte_and_lifetime_controls.rs"),
        "raw-byte strings, comments, and lifetimes are controls, stderr={stderr}"
    );
}

#[test]
fn rustdoc_fence_markers_and_executable_fences_are_custodied() {
    let temp = init_repo();
    let repo = temp.path();
    let ignored_tag = marker(&["ig", "nore"]);
    let no_run_tag = marker(&["no", "_", "run"]);

    write_file(
        repo,
        "tests/rustdoc_fences.rs",
        format!(
            "/// ```{ignored_tag}\n/// assert!(false);\n/// ```\n/// ```{no_run_tag}\n/// assert!(false);\n/// ```\n/// ```\n/// assert_eq!(2 + 2, 4);\n/// ```\npub fn documented() {{}}\n"
        ),
    );
    commit_all(repo, "add rustdoc fence probes");

    let marker_output = run_checker(repo, &["--check-markers"]);
    assert_failure_evidence(
        &marker_output,
        "Rustdoc ignore and no_run fences are live test-disable markers",
        &[
            "tests/rustdoc_fences.rs:1: rustdoc_ignore_fence",
            "tests/rustdoc_fences.rs:4: rustdoc_no_run_fence",
        ],
    );

    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    let generate = generate_baseline(repo, &baseline);
    assert_success(&generate, "baseline generation for rustdoc inventory probe");
    let stored = read_json(&baseline);
    assert!(
        stored["rustdoc_inventory"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "executable Rustdoc fences must be inventoried, baseline={stored:#}"
    );
}

#[test]
fn javascript_family_runtime_and_suite_markers_are_rejected() {
    let temp = init_repo();
    let repo = temp.path();
    let forms = [
        (
            "describe_skip.js",
            marker(&["describe", ".", "skip", "('x', () => {});"]),
            "javascript_suite_skip",
        ),
        (
            "context_skip.js",
            marker(&["context", ".", "skip", "('x', () => {});"]),
            "javascript_suite_skip",
        ),
        (
            "suite_skip.js",
            marker(&["suite", ".", "skip", "('x', () => {});"]),
            "javascript_suite_skip",
        ),
        (
            "test_only.js",
            marker(&["test", ".", "only", "('x', () => {});"]),
            "javascript_focus",
        ),
        (
            "it_todo.js",
            marker(&["it", ".", "todo", "('x');"]),
            "javascript_todo",
        ),
        (
            "skip_if.ts",
            marker(&["test", ".", "skipIf", "(true)('x', () => {});"]),
            "typescript_conditional_skip",
        ),
        (
            "run_if.ts",
            marker(&["it", ".", "runIf", "(false)('x', () => {});"]),
            "typescript_conditional_skip",
        ),
        (
            "x_alias.jsx",
            marker(&["x", "describe", "('x', () => {});"]),
            "jsx_disabled_alias",
        ),
        (
            "f_alias.tsx",
            marker(&["f", "it", "('x', () => {});"]),
            "tsx_focus_alias",
        ),
        (
            "pending.js",
            marker(&["pending", "();"]),
            "javascript_runtime_skip",
        ),
        (
            "this_skip.ts",
            marker(&["this", ".", "skip", "();"]),
            "typescript_runtime_skip",
        ),
    ];

    for (file, source, _) in &forms {
        write_file(repo, &format!("tests/{file}"), format!("{source}\n"));
    }
    commit_all(repo, "add JavaScript-family marker forms");

    let output = run_checker(repo, &["--check-markers"]);
    assert_failure_evidence(
        &output,
        "JavaScript-family suite, focus, todo, alias, and runtime markers",
        &forms
            .iter()
            .map(|(file, _, reason)| format!("tests/{file}:1: {reason}"))
            .collect::<Vec<_>>()
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>(),
    );
}

#[test]
fn javascript_regex_boundaries_and_template_interpolation_are_executable() {
    let clean = init_repo();
    let clean_repo = clean.path();
    let skipped = marker(&["test", ".", "skip"]);
    write_file(
        clean_repo,
        "tests/regex_control.js",
        format!("const pattern = /{skipped}\\('not live'\\)/;\n"),
    );
    commit_all(clean_repo, "add regex false-positive control");
    let clean_output = run_checker(clean_repo, &["--check-markers"]);
    assert_success(
        &clean_output,
        "marker-looking text inside a regular-expression literal is clean",
    );

    let live = init_repo();
    let live_repo = live.path();
    write_file(
        live_repo,
        "tests/regex_then_live.js",
        format!("const pattern = /[//]/;\n{skipped}('live', () => {{}});\n"),
    );
    write_file(
        live_repo,
        "tests/template_interpolation.js",
        format!("const rendered = `${{{skipped}('live', () => {{}})}}`;\n"),
    );
    commit_all(live_repo, "add regex and template false-negative probes");
    let live_output = run_checker(live_repo, &["--check-markers"]);
    assert_failure_evidence(
        &live_output,
        "regex lexing must not hide later code and template interpolation is executable",
        &[
            "tests/regex_then_live.js:2: javascript_test_skip",
            "tests/template_interpolation.js:1: javascript_test_skip",
        ],
    );
}

#[test]
fn python_runtime_decorator_unittest_and_f_string_markers_are_rejected() {
    let temp = init_repo();
    let repo = temp.path();
    let py = marker(&["pytest"]);
    let uni = marker(&["unittest"]);
    let content = format!(
        "import {py}\nimport {uni}\n\
         {py}.skip('runtime')\n\
         {py}.importorskip('missing_mod')\n\
         {py}.xfail('expected failure')\n\
         @{py}.mark.skipif(True, reason='skip')\n\
         def test_mark_skipif():\n    pass\n\
         @{py}.mark.xfail(reason='xfail')\n\
         def test_mark_xfail():\n    pass\n\
         @{uni}.skip('skip')\n\
         def test_unittest_skip():\n    pass\n\
         @{uni}.skipIf(True, 'skip')\n\
         def test_unittest_skip_if():\n    pass\n\
         @{uni}.skipUnless(False, 'skip')\n\
         def test_unittest_skip_unless():\n    pass\n\
         @{uni}.expectedFailure\n\
         def test_expected_failure():\n    pass\n\
         raise {uni}.SkipTest('skip')\n\
         msg = f\"{{{py}.skip('inside f-string')}}\"\n"
    );
    write_file(repo, "tests/python_markers.py", content);
    commit_all(repo, "add Python marker forms");

    let output = run_checker(repo, &["--check-markers"]);
    assert_failure_evidence(
        &output,
        "Python runtime, decorator, unittest, and f-string markers",
        &[
            "tests/python_markers.py:3: python_pytest_skip",
            "tests/python_markers.py:4: python_pytest_importorskip",
            "tests/python_markers.py:5: python_pytest_xfail",
            "tests/python_markers.py:6: python_pytest_mark_skipif",
            "tests/python_markers.py:9: python_pytest_mark_xfail",
            "tests/python_markers.py:12: python_unittest_skip",
            "tests/python_markers.py:15: python_unittest_skip_if",
            "tests/python_markers.py:18: python_unittest_skip_unless",
            "tests/python_markers.py:21: python_unittest_expected_failure",
            "tests/python_markers.py:24: python_unittest_skiptest",
            "tests/python_markers.py:25: python_f_string_skip",
        ],
    );
}

#[test]
fn mixed_rust_product_only_edits_pass_but_test_regions_and_modules_fail() {
    let temp = init_repo();
    let repo = temp.path();
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    write_file(
        repo,
        "flowspec/src/lib.rs",
        "pub fn product() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn inline_assertion() { assert_eq!(super::product(), 1); }\n}\n",
    );
    write_file(
        repo,
        "flowspec/src/external_tests.rs",
        "#[test]\nfn external_assertion() { assert_eq!(2 + 2, 4); }\n",
    );
    commit_all(repo, "add mixed Rust source");
    assert_success(&generate_baseline(repo, &baseline), "baseline generation");

    write_file(
        repo,
        "flowspec/src/lib.rs",
        "pub fn product() -> u8 { 2 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn inline_assertion() { assert_eq!(super::product(), 1); }\n}\n",
    );
    let product_only = verify_baseline(repo, &baseline);
    assert_success(
        &product_only,
        "product-only mixed Rust edits must not drift test custody",
    );

    write_file(
        repo,
        "flowspec/src/lib.rs",
        "pub fn product() -> u8 { 1 }\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn inline_assertion() { assert_eq!(super::product(), 2); }\n}\n",
    );
    let assertion_edit = verify_baseline(repo, &baseline);
    assert_failure_evidence(
        &assertion_edit,
        "inline Rust assertion edits remain sacred",
        &["rust_region_drift", "flowspec/src/lib.rs"],
    );
}

#[test]
fn whole_roots_harness_policy_and_include_assets_are_discovered() {
    let temp = init_repo();
    let repo = temp.path();
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");

    write_file(repo, "tests/oracle/case.yaml", "case: yaml\n");
    write_file(repo, "tests/oracle/case.json", "{\"case\":true}\n");
    write_file(repo, "tests/oracle/README.md", "# oracle\n");
    write_file(repo, "tests/snapshots/case.snap", "snapshot\n");
    write_file(repo, "tests/ui/case.stderr", "ui\n");
    write_file(repo, "tests/process-gates/evidence.md", "issue evidence\n");
    write_file(
        repo,
        "tests/test-custody-policy.json",
        "{\"schema_version\":1}\n",
    );
    write_file(repo, "tests/include-fixture.txt", "included fixture\n");
    write_file(
        repo,
        "flowspec-cli/tests/include_probe.rs",
        "const FIXTURE: &str = include_str!(\"../../tests/include-fixture.txt\");\n#[test]\nfn include_probe() { assert_eq!(FIXTURE.trim(), \"included fixture\"); }\n",
    );
    commit_all(repo, "add non-code assets and include edge");
    assert_success(&generate_baseline(repo, &baseline), "baseline generation");

    let stored = read_json(&baseline);
    let whole_files = stored["whole_files"]
        .as_object()
        .expect("schema v2 baseline must expose whole_files map");
    for expected in [
        "tests/oracle/case.yaml",
        "tests/oracle/case.json",
        "tests/oracle/README.md",
        "tests/snapshots/case.snap",
        "tests/ui/case.stderr",
        "tests/process-gates/evidence.md",
        "tests/test-custody-policy.json",
        "tests/include-fixture.txt",
        "scripts/check-test-custody.py",
        "scripts/flowspec-oracle.py",
        "scripts/validate-oracle-artifacts.py",
    ] {
        assert!(
            whole_files.contains_key(expected),
            "whole-file custody missing {expected}, baseline={stored:#}"
        );
    }
}

#[test]
fn baseline_binds_exact_cargo_and_rustdoc_inventory() {
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    let output = generate_baseline(&repo_root(), &baseline);
    assert_success(&output, "baseline generation for repository inventory");
    let stored = read_json(&baseline);

    assert_eq!(stored["schema_version"], 2);
    assert_eq!(
        stored["cargo_inventory"]["command"],
        "cargo test --workspace --all-features -- --list"
    );
    assert!(
        stored["cargo_inventory"]["executable_tests"]
            .as_array()
            .is_some_and(|items| items.len() >= 2369),
        "baseline must bind target-qualified executable identities, baseline={stored:#}"
    );
    assert!(
        stored["rustdoc_inventory"].as_array().is_some(),
        "baseline must explicitly record rustdoc identities, baseline={stored:#}"
    );
}

#[test]
fn baseline_schema_v2_rejects_malformed_partial_duplicate_and_phantom_entries() {
    let temp = init_repo();
    let repo = temp.path();
    write_file(repo, "tests/stable.rs", "#[test]\nfn stable() {}\n");
    let source_sha = commit_all(repo, "add stable test");
    let baseline_dir = TempDir::new().expect("baseline directory must be created");

    let cases = [
        ("non_object.json", "[]".to_owned(), "baseline_schema_type"),
        (
            "partial.json",
            format!(r#"{{"schema_version":2,"source_sha":"{source_sha}","whole_files":{{}}}}"#),
            "baseline_missing_required_key",
        ),
        (
            "duplicates.json",
            format!(
                r#"{{"schema_version":2,"source_sha":"{source_sha}","tracked_test_paths":["tests/stable.rs","tests/stable.rs"],"files":{{"tests/stable.rs":"00"}}}}"#
            ),
            "baseline_duplicate_path",
        ),
        (
            "phantom.json",
            format!(
                r#"{{"schema_version":2,"source_sha":"{source_sha}","tracked_test_paths":["tests/stable.rs"],"files":{{"tests/stable.rs":"00","tests/phantom.rs":"00"}}}}"#
            ),
            "baseline_phantom_path",
        ),
        (
            "noncanonical.json",
            format!(
                r#"{{"schema_version":2,"source_sha":"{source_sha}","tracked_test_paths":["tests/../tests/stable.rs"],"files":{{"tests/../tests/stable.rs":"00"}}}}"#
            ),
            "baseline_noncanonical_path",
        ),
    ];

    for (file, content, reason) in cases {
        let path = baseline_dir.path().join(file);
        fs::write(&path, content).expect("baseline fixture must be written");
        let output = verify_baseline(repo, &path);
        assert_failure_evidence(
            &output,
            &format!("schema-v2 malformed case {file}"),
            &[reason],
        );
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            !stderr.contains("Traceback"),
            "schema failures must use stable evidence, stderr={stderr}"
        );
    }
}

#[test]
fn baseline_rejects_forged_hashes_against_source_git_objects() {
    let temp = init_repo();
    let repo = temp.path();
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    let relative = "tests/stable.rs";

    write_file(
        repo,
        relative,
        "#[test]\nfn stable() { assert_eq!(1, 1); }\n",
    );
    let source_sha = commit_all(repo, "add source authority");
    assert_success(&generate_baseline(repo, &baseline), "baseline generation");

    write_file(
        repo,
        relative,
        "#[test]\nfn stable() { assert_eq!(2, 2); }\n",
    );
    commit_all(repo, "change sacred bytes");
    let forged_hash = sha256_file(&repo.join(relative));
    let original = fs::read_to_string(&baseline).expect("baseline must be readable");
    let rewritten = original
        .lines()
        .map(|line| {
            if line.contains(relative) && line.contains(':') {
                format!("    \"{relative}\": \"{forged_hash}\"")
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&baseline, rewritten).expect("forged baseline must be written");

    let output = verify_baseline(repo, &baseline);
    assert_failure_evidence(
        &output,
        "stored hashes must be checked against source_sha Git objects, not only worktree bytes",
        &[
            "baseline_source_mismatch",
            "stored_hash_forged",
            &source_sha,
        ],
    );
}

fn sha256_file(path: &Path) -> String {
    let output = Command::new("sha256sum")
        .arg(path)
        .output()
        .expect("sha256sum must execute");
    assert!(
        output.status.success(),
        "sha256sum failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("sha256sum output must be UTF-8")
        .split_whitespace()
        .next()
        .expect("sha256sum must print digest")
        .to_owned()
}

#[cfg(unix)]
#[test]
fn hostile_paths_and_nonregular_sources_fail_with_deterministic_evidence() {
    let temp = init_repo();
    let repo = temp.path();
    write_file(repo, "tests/missing.py", "def test_missing():\n    pass\n");
    write_file(
        repo,
        "tests/final_symlink.py",
        "def test_link():\n    pass\n",
    );
    write_file(
        repo,
        "tests/directory.py",
        "def test_directory():\n    pass\n",
    );
    write_file(repo, "parent/file.py", "def test_parent():\n    pass\n");
    write_file(
        repo,
        "tests/mode_zero.py",
        "def test_mode_zero():\n    pass\n",
    );
    write_file(
        repo,
        "tests/invalid_content.py",
        b"def test_bad():\n    bad = '\xff'\n",
    );
    fs::create_dir_all(repo.join("tests")).expect("tests directory must exist");
    let invalid_path = b"tests/invalid_path_\xff.py";
    let invalid_os = std::ffi::OsString::from_vec(invalid_path.to_vec());
    fs::write(
        repo.join(PathBuf::from(invalid_os)),
        "def test_path():\n    pass\n",
    )
    .expect("invalid-UTF-8 path fixture must be written");
    git(
        repo,
        &[
            "add",
            "tests/missing.py",
            "tests/final_symlink.py",
            "tests/directory.py",
        ],
    );
    git(
        repo,
        &[
            "add",
            "parent/file.py",
            "tests/mode_zero.py",
            "tests/invalid_content.py",
        ],
    );
    git_add_path_bytes(repo, invalid_path);
    git(repo, &["commit", "-m", "add hostile source fixtures"]);

    fs::remove_file(repo.join("tests/missing.py")).expect("missing fixture must be removed");
    fs::remove_file(repo.join("tests/final_symlink.py")).expect("link fixture must be replaced");
    symlink("../parent/file.py", repo.join("tests/final_symlink.py"))
        .expect("final symlink must be created");
    fs::remove_file(repo.join("tests/directory.py")).expect("directory fixture must be replaced");
    fs::create_dir(repo.join("tests/directory.py")).expect("directory fixture must be created");
    fs::rename(repo.join("parent"), repo.join("real_parent")).expect("parent must move");
    symlink("real_parent", repo.join("parent")).expect("symlink parent must be created");
    fs::set_permissions(
        repo.join("tests/mode_zero.py"),
        fs::Permissions::from_mode(0o000),
    )
    .expect("mode-zero fixture must be created");

    let first = run_checker(repo, &["--check-markers"]);
    let second = run_checker(repo, &["--check-markers"]);
    fs::set_permissions(
        repo.join("tests/mode_zero.py"),
        fs::Permissions::from_mode(0o644),
    )
    .expect("mode-zero fixture permissions must be restored");
    assert_eq!(
        first.stderr, second.stderr,
        "hostile-source evidence must be deterministic across twin runs"
    );
    assert_failure_evidence(
        &first,
        "hostile source and path handling",
        &[
            "tests/missing.py:0: source_missing",
            "tests/final_symlink.py:0: source_final_symlink",
            "tests/directory.py:0: source_not_regular",
            "parent/file.py:0: source_symlink_component",
            "tests/mode_zero.py:0: source_mode_unreadable",
            "tests/invalid_content.py:0: invalid_content_utf8",
            "invalid_path_ff.py:0: invalid_path_utf8",
        ],
    );
}

#[cfg(unix)]
#[test]
fn fifo_sources_fail_closed_without_blocking() {
    let temp = init_repo();
    let repo = temp.path();
    let relative = "tests/fifo.py";
    write_file(repo, relative, "def test_fifo():\n    pass\n");
    commit_all(repo, "add fifo path as regular source");
    fs::remove_file(repo.join(relative)).expect("regular source must be replaced");
    let status = Command::new("mkfifo")
        .arg(repo.join(relative))
        .status()
        .expect("mkfifo must execute");
    assert!(status.success(), "mkfifo must create FIFO fixture");

    let output = run_checker_with_timeout(repo, &["--check-markers"], Duration::from_secs(2))
        .expect("checker must not block on a FIFO");
    assert_failure_evidence(
        &output,
        "tracked FIFO source must fail closed without blocking",
        &["tests/fifo.py:0: source_not_regular_fifo"],
    );
}

#[test]
fn baseline_generation_is_atomic_and_refuses_dirty_or_lossy_authority() {
    let temp = init_repo();
    let repo = temp.path();
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    fs::write(&baseline, "sentinel baseline\n").expect("sentinel baseline must be written");
    write_file(repo, "tests/stable.rs", "#[test]\nfn stable() {}\n");
    commit_all(repo, "add stable test");
    write_file(repo, "tests/stable.rs", "#[test]\nfn dirty() {}\n");

    let output = generate_baseline(repo, &baseline);
    assert_failure_evidence(
        &output,
        "baseline generation must refuse dirty source authority",
        &["baseline_generation_dirty_source"],
    );
    assert_eq!(
        fs::read_to_string(&baseline).expect("baseline must remain readable"),
        "sentinel baseline\n",
        "failed baseline generation must leave destination untouched"
    );
}

#[test]
fn checker_rejects_action_ambiguity_and_noop_self_test() {
    let temp = init_repo();
    let repo = temp.path();
    let baseline_dir = TempDir::new().expect("baseline directory must be created");
    let baseline = baseline_dir.path().join("baseline.json");
    write_file(repo, "tests/stable.rs", "#[test]\nfn stable() {}\n");
    commit_all(repo, "add stable test");
    assert_success(&generate_baseline(repo, &baseline), "baseline generation");

    let ambiguous = run_checker(
        repo,
        &[
            "--check-markers",
            "--baseline",
            baseline.to_str().expect("baseline path must be UTF-8"),
        ],
    );
    assert_failure_evidence(
        &ambiguous,
        "conflicting primary actions must be rejected",
        &["action_ambiguous"],
    );

    let self_test = run_checker(repo, &["--self-test"]);
    assert_failure_evidence(
        &self_test,
        "banner-only self-test is not custody evidence",
        &["self_test_noop_rejected"],
    );
}
