//! Cycle 17 — QA-3 (QA-Surface) Tests: `init` command implementation.
//!
//! 25 tests validating the `flowspec init` command: config creation, language
//! detection, no-overwrite safety, exit codes, pipe safety, and regressions.

use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn flowspec() -> Command {
    Command::cargo_bin("flowspec").unwrap()
}

fn workspace_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

// ==========================================================================
// Category 1: TDD Anchors — Fresh Init (T1–T5)
// ==========================================================================

/// T1: flowspec init creates .flowspec/config.yaml
#[test]
fn t01_init_creates_config_file() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let output = flowspec().args(["init", dir_path]).output().unwrap();

    let config_path = dir.path().join(".flowspec").join("config.yaml");
    assert!(
        config_path.exists(),
        "flowspec init must create .flowspec/config.yaml.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T2: flowspec init creates .flowspec/ directory
#[test]
fn t02_init_creates_flowspec_directory() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    flowspec().args(["init", dir_path]).output().unwrap();

    let flowspec_dir = dir.path().join(".flowspec");
    assert!(
        flowspec_dir.is_dir(),
        ".flowspec/ directory must exist after init"
    );
}

/// T3: flowspec init exits 0 on success
#[test]
fn t03_init_exits_zero_on_success() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let output = flowspec().args(["init", dir_path]).output().unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "flowspec init must exit 0 on successful config creation.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T4: flowspec init prints config to stdout
#[test]
fn t04_init_prints_config_to_stdout() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let output = flowspec().args(["init", dir_path]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Spec: "Prints the generated config to stdout"
    assert!(
        stdout.contains("languages") || stdout.contains("exclude"),
        "flowspec init must print generated config to stdout.\nGot stdout: '{}'",
        stdout
    );
}

/// T5: flowspec init config contains valid YAML
#[test]
fn t05_init_stdout_is_valid_yaml() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    let output = flowspec().args(["init", dir_path]).output().unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Must parse as YAML without error
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout from init must be valid YAML.\nGot: '{}'\nError: {:?}",
        stdout,
        parsed.err()
    );
}

// ==========================================================================
// Category 2: TDD Anchors — Language Detection (T6–T8)
// ==========================================================================

/// T6: Detects Python files in project directory
#[test]
fn t06_init_detects_python() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def main(): pass").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("python"),
        "init must detect .py files and include 'python' in config.\nGot: {}",
        stdout
    );
}

/// T7: Detects JavaScript/TypeScript files
#[test]
fn t07_init_detects_javascript_typescript() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("index.js"), "function f() {}").unwrap();
    fs::write(dir.path().join("types.ts"), "interface Foo {}").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("javascript") || stdout.contains("typescript"),
        "init must detect .js/.ts files.\nGot: {}",
        stdout
    );
}

/// T8: Detects Rust files
#[test]
fn t08_init_detects_rust() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn main() {}").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("rust"),
        "init must detect .rs files.\nGot: {}",
        stdout
    );
}

// ==========================================================================
// Category 3: TDD Anchors — Existing Config (T9–T10)
// ==========================================================================

/// T9: flowspec init with existing config exits 0
#[test]
fn t09_init_existing_config_exits_zero() {
    let dir = TempDir::new().unwrap();
    let flowspec_dir = dir.path().join(".flowspec");
    fs::create_dir_all(&flowspec_dir).unwrap();
    fs::write(flowspec_dir.join("config.yaml"), "languages:\n  - python\n").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "init with existing config must exit 0, not error.\nstderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// T10: flowspec init with existing config does NOT overwrite
#[test]
fn t10_init_existing_config_no_overwrite() {
    let dir = TempDir::new().unwrap();
    let flowspec_dir = dir.path().join(".flowspec");
    fs::create_dir_all(&flowspec_dir).unwrap();
    let config_path = flowspec_dir.join("config.yaml");
    let original_content = "languages:\n  - custom_lang\n";
    fs::write(&config_path, original_content).unwrap();

    flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let after = fs::read_to_string(&config_path).unwrap();
    assert_eq!(
        after, original_content,
        "init must NOT overwrite existing config.yaml.\nOriginal: '{}'\nAfter: '{}'",
        original_content, after
    );
}

// ==========================================================================
// Category 4: Adversarial Edge Cases (T11–T17)
// ==========================================================================

/// T11: Non-existent path exits 1
#[test]
fn t11_init_nonexistent_path_exits_one() {
    let output = flowspec()
        .args(["init", "/tmp/flowspec_nonexistent_path_abc123"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "init on nonexistent path must exit 1"
    );
}

/// T12: Path is a file, not a directory
#[test]
fn t12_init_path_is_file_exits_one() {
    let dir = TempDir::new().unwrap();
    let file_path = dir.path().join("not_a_dir.txt");
    fs::write(&file_path, "content").unwrap();

    let output = flowspec()
        .args(["init", file_path.to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        1,
        "init on a file (not directory) must exit 1"
    );
}

/// T13: Double init is idempotent (no-op, exit 0)
#[test]
fn t13_double_init_idempotent() {
    let dir = TempDir::new().unwrap();
    let dir_path = dir.path().to_str().unwrap();

    // First init
    let first = flowspec().args(["init", dir_path]).output().unwrap();
    let first_config =
        fs::read_to_string(dir.path().join(".flowspec").join("config.yaml")).unwrap();

    // Second init
    let second = flowspec().args(["init", dir_path]).output().unwrap();
    let second_config =
        fs::read_to_string(dir.path().join(".flowspec").join("config.yaml")).unwrap();

    assert_eq!(first.status.code().unwrap(), 0, "first init must exit 0");
    assert_eq!(second.status.code().unwrap(), 0, "second init must exit 0");
    assert_eq!(
        first_config, second_config,
        "config must not change on second init"
    );
}

/// T14: Empty project (no source files) still creates config
#[test]
fn t14_init_empty_project_creates_config() {
    let dir = TempDir::new().unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "empty project init must exit 0"
    );
    assert!(
        dir.path().join(".flowspec").join("config.yaml").exists(),
        "config must be created even with no source files"
    );
}

/// T15: Existing empty config file treated as existing
#[test]
fn t15_init_empty_config_file_no_overwrite() {
    let dir = TempDir::new().unwrap();
    let flowspec_dir = dir.path().join(".flowspec");
    fs::create_dir_all(&flowspec_dir).unwrap();
    fs::write(flowspec_dir.join("config.yaml"), "").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code().unwrap(),
        0,
        "empty config file must exit 0"
    );
    let content = fs::read_to_string(flowspec_dir.join("config.yaml")).unwrap();
    assert_eq!(content, "", "empty config must NOT be overwritten");
}

/// T16: Corrupted config file treated as existing
#[test]
fn t16_init_corrupted_config_no_overwrite() {
    let dir = TempDir::new().unwrap();
    let flowspec_dir = dir.path().join(".flowspec");
    fs::create_dir_all(&flowspec_dir).unwrap();
    let bad_yaml = "{{{{not yaml at all::::";
    fs::write(flowspec_dir.join("config.yaml"), bad_yaml).unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    assert_eq!(output.status.code().unwrap(), 0);
    let content = fs::read_to_string(flowspec_dir.join("config.yaml")).unwrap();
    assert_eq!(
        content, bad_yaml,
        "corrupted config must NOT be overwritten"
    );
}

/// T17: Language detection excludes node_modules/ and target/
#[test]
fn t17_init_excludes_generated_directories() {
    let dir = TempDir::new().unwrap();
    // Only JS files are in node_modules — should NOT be detected
    let nm = dir.path().join("node_modules").join("pkg");
    fs::create_dir_all(&nm).unwrap();
    fs::write(nm.join("index.js"), "module.exports = {}").unwrap();
    // Only Rust files are in target/
    let target = dir.path().join("target").join("debug");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("main.rs"), "fn main() {}").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // With no source files outside excluded dirs, languages should be empty
    assert!(
        !stdout.contains("javascript") && !stdout.contains("rust"),
        "init must not detect languages from node_modules/ or target/.\nGot: {}",
        stdout
    );
}

// ==========================================================================
// Category 5: Exit Code Contract (T18–T19)
// ==========================================================================

/// T18: Init never exits 2
#[test]
fn t18_init_never_exits_two() {
    // Fresh dir
    let dir = TempDir::new().unwrap();
    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert_ne!(
        output.status.code().unwrap(),
        2,
        "init must never exit 2 (no findings concept)"
    );

    // Non-existent dir
    let output2 = flowspec()
        .args(["init", "/tmp/flowspec_no_such_dir_xyz"])
        .output()
        .unwrap();
    assert_ne!(
        output2.status.code().unwrap(),
        2,
        "init error must exit 1, never 2"
    );
}

/// T19: Pipe safety — no log/tracing output on stdout
#[test]
fn t19_init_stdout_pipe_safe() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "x = 1").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    // No tracing prefixes on stdout
    assert!(
        !stdout.contains("INFO") && !stdout.contains("WARN") && !stdout.contains("DEBUG"),
        "stdout must be pipe-safe (no tracing output).\nGot: {}",
        stdout
    );
}

// ==========================================================================
// Category 6: Regression Guards (T20–T23)
// ==========================================================================

/// T20: watch still returns CommandNotImplemented (diff implemented in C18)
#[test]
fn t20_diff_watch_still_unimplemented() {
    // diff is implemented as of C18 — only watch remains deferred
    for (cmd, args) in &[("watch", vec!["watch"])] {
        let output = flowspec().args(args).output().unwrap();
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("not yet implemented"),
            "{} must still return 'not yet implemented'.\nGot: {}",
            cmd,
            stderr
        );
    }
}

/// T21: analyze command unaffected by init changes
#[test]
fn t21_analyze_still_works() {
    let output = flowspec()
        .args(["analyze", ".", "--format", "summary"])
        .current_dir(workspace_root())
        .output()
        .unwrap();

    assert!(
        output.status.code().unwrap() == 0 || output.status.code().unwrap() == 2,
        "analyze must exit 0 or 2, got {}",
        output.status.code().unwrap()
    );
}

/// T22: C15 convergence byte floor still enforced
#[test]
fn t22_manifest_byte_floor_still_enforced() {
    let dir = TempDir::new().unwrap();
    // Tiny file — should trigger byte floor protection
    fs::write(dir.path().join("tiny.py"), "x = 1").unwrap();
    let output = flowspec()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "yaml"])
        .output()
        .unwrap();

    // Must not error from manifest size validation
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.contains("manifest too large"),
        "byte floor must protect small files from 10x ratio violation"
    );
    assert_eq!(output.status.code().unwrap(), 0);
}

/// T23: 8-section manifest structure preserved
#[test]
fn t23_manifest_eight_sections() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def f():\n    return 1\n").unwrap();

    let output = flowspec()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let json: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let obj = json.as_object().unwrap();

    for section in &[
        "metadata",
        "summary",
        "entities",
        "diagnostics",
        "flows",
        "boundaries",
        "type_flows",
        "dependency_graph",
    ] {
        assert!(
            obj.contains_key(*section),
            "manifest must contain '{}' section",
            section
        );
    }
}

// ==========================================================================
// Category 7: Config Content Validation (T24–T25)
// ==========================================================================

/// T24: Generated config includes exclude patterns
#[test]
fn t24_init_config_has_exclude_patterns() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "x = 1").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("exclude"),
        "config must have exclude section"
    );
    assert!(
        stdout.contains("node_modules") && stdout.contains("target"),
        "exclude patterns must include node_modules/ and target/.\nGot: {}",
        stdout
    );
}

/// T25: Written config file matches stdout
#[test]
fn t25_init_file_matches_stdout() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("lib.rs"), "pub fn f() {}").unwrap();

    let output = flowspec()
        .args(["init", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let file_content =
        fs::read_to_string(dir.path().join(".flowspec").join("config.yaml")).unwrap();

    // Strip comments for comparison (stdout and file should have same YAML content)
    let stdout_trimmed = stdout.trim();
    let file_trimmed = file_content.trim();
    assert_eq!(
        stdout_trimmed, file_trimmed,
        "config file on disk must match stdout output"
    );
}
