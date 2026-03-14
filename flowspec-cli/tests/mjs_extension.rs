//! Tests for .mjs (ES module JavaScript) extension support.
//!
//! The bug: `discover_source_files()` matched `"js" | "jsx"` but not `"mjs"`.
//! ES module JavaScript files were silently skipped during analysis.

use std::process::Command;

/// Get the flowspec binary path.
fn flowspec_bin() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flowspec"))
}

#[test]
fn mjs_files_discovered_and_analyzed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("module.mjs"),
        "export function hello() { return 42; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args(["--format", "json", "analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Invalid JSON output: {}\nstdout: {}", e, stdout));

    // .mjs file should produce entities
    let entities = parsed["entities"].as_array().unwrap();
    assert!(
        !entities.is_empty(),
        ".mjs files must be discovered and produce entities"
    );

    // metadata.languages should include "javascript"
    let languages = parsed["metadata"]["languages"].as_array().unwrap();
    let lang_strs: Vec<&str> = languages.iter().filter_map(|v| v.as_str()).collect();
    assert!(
        lang_strs.contains(&"javascript"),
        "metadata.languages should include 'javascript' for .mjs files, got: {:?}",
        lang_strs
    );
}

#[test]
fn mjs_files_counted_in_metadata() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("utils.mjs"),
        "export function helper() { return 'help'; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args(["--format", "json", "analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let file_count = parsed["metadata"]["file_count"].as_u64().unwrap();
    assert!(
        file_count >= 1,
        "File count must reflect .mjs files, got: {}",
        file_count
    );

    let entity_count = parsed["metadata"]["entity_count"].as_u64().unwrap();
    assert!(
        entity_count > 0,
        "Entity count must be > 0 for .mjs file, got: {}",
        entity_count
    );
}

#[test]
fn mjs_included_with_language_javascript() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("main.js"),
        "function mainFn() { return 1; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("utils.mjs"),
        "export function utilFn() { return 2; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "analyze",
            "--language",
            "javascript",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let file_count = parsed["metadata"]["file_count"].as_u64().unwrap();
    assert!(
        file_count >= 2,
        "Both .js and .mjs files should be included with --language javascript, got file_count: {}",
        file_count
    );
}

#[test]
fn mjs_excluded_with_language_python() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("main.py"), "def main():\n    pass\n").unwrap();
    std::fs::write(
        dir.path().join("utils.mjs"),
        "export function utilFn() { return 2; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args([
            "--format",
            "json",
            "analyze",
            "--language",
            "python",
            dir.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    // No entities from .mjs files
    let entities = parsed["entities"].as_array().unwrap();
    for entity in entities {
        let loc = entity["loc"].as_str().unwrap_or("");
        assert!(
            !loc.contains(".mjs"),
            ".mjs entities should not appear with --language python, got loc: {}",
            loc
        );
    }
}

#[test]
fn mjs_only_directory_produces_output() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.mjs"),
        "export function main() { return 'hello'; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("utils.mjs"),
        "export function helper() { return 'world'; }\n",
    )
    .unwrap();

    let output = flowspec_bin()
        .args(["--format", "json", "analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let entities = parsed["entities"].as_array().unwrap();
    assert!(
        !entities.is_empty(),
        "A project with only .mjs files must not appear empty"
    );

    let exit_code = output.status.code().unwrap();
    assert!(
        exit_code == 0 || exit_code == 2,
        "Exit code should be 0 (clean) or 2 (findings), got: {}",
        exit_code
    );
}
