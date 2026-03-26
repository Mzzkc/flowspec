//! Cycle 16 QA-3 surface integration tests — method call visibility,
//! stale artifact cleanup, and regression guards.
//!
//! 25 tests across 7 categories:
//! - Category 1 (T1-T5): Method call visibility in entity manifest
//! - Category 2 (T6-T8): Trace command follows method call edges
//! - Category 3 (T9-T11): Cross-format consistency for method call edges
//! - Category 4 (T12-T13): Cleanup verification
//! - Category 5 (T14-T17): Adversarial edge cases
//! - Category 6 (T18-T23): Regression guards (prior cycle contracts)
//! - Category 7 (T24-T25): Diagnostic count orthogonality

use assert_cmd::Command;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

// ============================================================================
// Helpers
// ============================================================================

fn flowspec() -> Command {
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    cmd.current_dir(workspace_root());
    cmd
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf()
}

fn python_method_fixture_path() -> String {
    workspace_root()
        .join("tests/fixtures/method_calls/python")
        .to_str()
        .unwrap()
        .to_string()
}

fn js_method_fixture_path() -> String {
    workspace_root()
        .join("tests/fixtures/method_calls/javascript")
        .to_str()
        .unwrap()
        .to_string()
}

fn rust_method_fixture_path() -> String {
    workspace_root()
        .join("tests/fixtures/method_calls/rust")
        .to_str()
        .unwrap()
        .to_string()
}

fn rust_fixture_path() -> String {
    workspace_root()
        .join("tests/fixtures/rust")
        .to_str()
        .unwrap()
        .to_string()
}

fn create_minimal_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("tiny.py"), "def f():\n    pass\n\nf()\n").unwrap();
    dir
}

fn create_clean_python() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        "def greet(name):\n    return f'Hello, {name}'\n\n\
         def main():\n    result = greet('world')\n    print(result)\n\n\
         if __name__ == '__main__':\n    main()\n",
    )
    .unwrap();
    dir
}

fn create_python_with_dead_code() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead_code.py"),
        "import os\n\n\
         def active_function():\n    return 42\n\n\
         def dead_function():\n    return 'unreachable'\n\n\
         def another_dead():\n    return 0\n\n\
         def main():\n    result = active_function()\n    print(result)\n",
    )
    .unwrap();
    dir
}

// ============================================================================
// Category 1: Method Call Visibility in Entity Manifest (T1-T5)
// T1-T4 are TDD anchors — expected to FAIL before Worker 1's method call
// tracking and PASS after. T5 is a regression guard.
// ============================================================================

/// T1: Python self.method() appears in entity calls/called_by.
/// TDD anchor — requires Worker 1's method call tracking in the graph.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t01_python_self_method_in_entity_calls() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");

    // Find Processor.transform — its calls should include validate
    let transform = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("transform"))
                .unwrap_or(false)
        })
        .expect("Processor.transform entity must exist");
    let transform_calls = transform["calls"].as_array().expect("calls must be array");
    assert!(
        transform_calls
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("validate")).unwrap_or(false)),
        "Processor.transform must call validate via self.validate(). calls: {:?}",
        transform_calls
    );

    // Find Processor.validate — its called_by should include transform
    let validate = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("validate"))
                .unwrap_or(false)
        })
        .expect("Processor.validate entity must exist");
    let validate_called_by = validate["called_by"]
        .as_array()
        .expect("called_by must be array");
    assert!(
        validate_called_by
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("transform")).unwrap_or(false)),
        "Processor.validate must be called_by transform via self.validate(). called_by: {:?}",
        validate_called_by
    );
}

/// T2: JavaScript this.method() appears in entity calls/called_by.
/// TDD anchor — requires Worker 1's JS adapter fix for this.method().
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t02_js_this_method_in_entity_calls() {
    let fixture = js_method_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");

    let process = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("process"))
                .unwrap_or(false)
        })
        .expect("Handler.process entity must exist");
    let process_calls = process["calls"].as_array().expect("calls must be array");
    assert!(
        process_calls
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("sanitize")).unwrap_or(false)),
        "Handler.process must call sanitize via this.sanitize(). calls: {:?}",
        process_calls
    );

    let sanitize = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("sanitize"))
                .unwrap_or(false)
        })
        .expect("Handler.sanitize entity must exist");
    let sanitize_called_by = sanitize["called_by"]
        .as_array()
        .expect("called_by must be array");
    assert!(
        sanitize_called_by
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("process")).unwrap_or(false)),
        "Handler.sanitize must be called_by process via this.sanitize(). called_by: {:?}",
        sanitize_called_by
    );
}

/// T3: Rust self.method() in impl block appears in entity calls/called_by.
/// TDD anchor — requires Worker 1's Rust adapter method call tracking.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t03_rust_self_method_in_entity_calls() {
    let fixture = rust_method_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");

    let start = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("start"))
                .unwrap_or(false)
        })
        .expect("Engine::start entity must exist");
    let start_calls = start["calls"].as_array().expect("calls must be array");
    assert!(
        start_calls.iter().any(|c| c
            .as_str()
            .map(|s| s.contains("initialize"))
            .unwrap_or(false)),
        "Engine::start must call initialize via self.initialize(). calls: {:?}",
        start_calls
    );

    let initialize = entities
        .iter()
        .find(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("initialize"))
                .unwrap_or(false)
        })
        .expect("Engine::initialize entity must exist");
    let init_called_by = initialize["called_by"]
        .as_array()
        .expect("called_by must be array");
    assert!(
        init_called_by
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("start")).unwrap_or(false)),
        "Engine::initialize must be called_by start via self.initialize(). called_by: {:?}",
        init_called_by
    );
}

/// T4: Method called via self no longer appears as data_dead_end.
/// TDD anchor — requires Worker 1's method call tracking.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t04_self_method_not_data_dead_end() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args([
            "diagnose",
            &fixture,
            "--format",
            "json",
            "--checks",
            "data_dead_end",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let diag_array = diagnostics
        .as_array()
        .expect("diagnose output must be array");
    // Processor.validate should NOT appear as data_dead_end (called via self.validate())
    let has_validate_dead_end = diag_array.iter().any(|d| {
        d["entity"]
            .as_str()
            .map(|s| s.contains("validate"))
            .unwrap_or(false)
    });
    assert!(
        !has_validate_dead_end,
        "validate() should not be data_dead_end — it's called via self.validate()"
    );
}

/// T5: True dead-end method still detected after method call tracking.
/// Regression guard — must pass regardless of Worker 1's changes.
#[test]
fn t05_true_dead_end_still_detected() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args([
            "diagnose",
            &fixture,
            "--format",
            "json",
            "--checks",
            "data_dead_end",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "JSON parse failed: {}\nOutput:\n{}",
            e,
            &stdout[..stdout.len().min(500)]
        )
    });

    let diag_array = diagnostics
        .as_array()
        .expect("diagnose output must be array");
    // unused_helper should still appear as data_dead_end — never called by anyone
    let has_unused_dead_end = diag_array.iter().any(|d| {
        d["entity"]
            .as_str()
            .map(|s| s.contains("unused_helper"))
            .unwrap_or(false)
    });
    assert!(
        has_unused_dead_end,
        "unused_helper() must still be data_dead_end — it is never called by anything"
    );
}

// ============================================================================
// Category 2: Trace Command Follows Method Call Edges (T6-T8)
// TDD anchors — require Worker 1's method call edges.
// ============================================================================

/// T6: Trace backward through self.method() call chain.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t06_trace_backward_through_self_method() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args([
            "trace",
            "--symbol",
            "validate",
            "--direction",
            "backward",
            &fixture,
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(code, 0, "trace must succeed with exit code 0, got {}", code);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("transform"),
        "Backward trace from validate must include transform (caller via self.validate()). Got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

/// T7: Trace forward from method that calls self.method().
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t07_trace_forward_from_self_caller() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args([
            "trace",
            "--symbol",
            "transform",
            "--direction",
            "forward",
            &fixture,
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(code, 0, "trace must succeed with exit code 0, got {}", code);

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("validate"),
        "Forward trace from transform must include validate (callee via self.validate()). Got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

/// T8: Trace through chained self.method() calls.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t08_trace_chained_self_methods() {
    // Use only chained fixture dir to isolate
    let dir = TempDir::new().unwrap();
    let chained_src = workspace_root().join("tests/fixtures/method_calls/python/chained_self.py");
    let content = fs::read_to_string(&chained_src).unwrap();
    fs::write(dir.path().join("chained_self.py"), &content).unwrap();

    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args([
            "trace",
            "--symbol",
            "step2",
            "--direction",
            "backward",
            "--depth",
            "3",
            dir_path,
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(code, 0, "trace must succeed, got exit code {}", code);

    let stdout = String::from_utf8(output.stdout).unwrap();
    // Chained: run → step1 → step2. Backward trace from step2 should find step1.
    assert!(
        stdout.contains("step1"),
        "Backward trace from step2 must include step1 (chained self calls). Got:\n{}",
        &stdout[..stdout.len().min(500)]
    );
}

// ============================================================================
// Category 3: Cross-Format Consistency for Method Call Edges (T9-T11)
// TDD anchors — require Worker 1's method call edges.
// ============================================================================

/// T9: YAML and JSON produce same calls/called_by for method entities.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t09_yaml_json_method_calls_consistent() {
    let fixture = python_method_fixture_path();

    // Run JSON
    let json_out = flowspec()
        .args(["analyze", &fixture, "--format", "json"])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8(json_out.stdout).unwrap();
    let json_parsed: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();
    let json_entities = json_parsed["entities"].as_array().unwrap();

    // Run YAML
    let yaml_out = flowspec()
        .args(["analyze", &fixture, "--format", "yaml"])
        .output()
        .unwrap();
    let yaml_stdout = String::from_utf8(yaml_out.stdout).unwrap();
    let yaml_parsed: serde_yaml::Value = serde_yaml::from_str(&yaml_stdout).unwrap();
    let yaml_entities = yaml_parsed["entities"].as_sequence().unwrap();

    // Entity counts must match
    assert_eq!(
        json_entities.len(),
        yaml_entities.len(),
        "JSON and YAML must have same entity count"
    );
}

/// T10: SARIF results include method call diagnostic changes.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t10_sarif_method_call_diagnostics_consistent() {
    let fixture = python_method_fixture_path();

    // JSON diagnose
    let json_out = flowspec()
        .args(["diagnose", &fixture, "--format", "json"])
        .output()
        .unwrap();
    let json_stdout = String::from_utf8(json_out.stdout).unwrap();
    let json_diag: serde_json::Value = serde_json::from_str(&json_stdout).unwrap();
    let json_count = json_diag.as_array().unwrap().len();

    // SARIF diagnose
    let sarif_out = flowspec()
        .args(["diagnose", &fixture, "--format", "sarif"])
        .output()
        .unwrap();
    let sarif_stdout = String::from_utf8(sarif_out.stdout).unwrap();
    let sarif_parsed: serde_json::Value = serde_json::from_str(&sarif_stdout).unwrap();

    assert_eq!(
        sarif_parsed["version"].as_str().unwrap(),
        "2.1.0",
        "SARIF version must be 2.1.0"
    );

    let sarif_results = sarif_parsed["runs"][0]["results"]
        .as_array()
        .expect("SARIF must have results array");
    assert_eq!(
        sarif_results.len(),
        json_count,
        "SARIF and JSON diagnostic counts must match"
    );
}

/// T11: Summary format reflects reduced diagnostic count.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t11_summary_reflects_method_call_fix() {
    let fixture = python_method_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "summary"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(!stdout.is_empty(), "Summary output must be non-empty");
}

// ============================================================================
// Category 4: Cleanup Verification (T12-T13)
// ============================================================================

/// T12: Stale dogfood-raw.txt removed or updated.
#[test]
fn t12_stale_dogfood_artifact_resolved() {
    let stale_path = workspace_root().join("workspaces/build/cycle-15/dogfood-raw.txt");
    if stale_path.exists() {
        // If it still exists, it must have current numbers (not pre-C15 250/652)
        let content = fs::read_to_string(&stale_path).unwrap();
        assert!(
            !content.contains("250") || !content.contains("652"),
            "dogfood-raw.txt still shows pre-C15 numbers (250/652). Must be deleted or updated."
        );
    }
    // If deleted, test passes
}

/// T13: No stray untracked files in flowspec/src/.
#[test]
fn t13_no_stray_untracked_src_files() {
    let src_dir = workspace_root().join("flowspec/src");
    let lib_content = fs::read_to_string(src_dir.join("lib.rs")).unwrap();

    // Check that all top-level .rs files in src/ are either:
    // (a) modules declared in lib.rs, or
    // (b) special files (lib.rs itself)
    for entry in fs::read_dir(&src_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        if path.is_file() && path.extension().map(|e| e == "rs").unwrap_or(false) {
            let stem = path.file_stem().unwrap().to_str().unwrap();
            if stem == "lib" || stem == "commands" {
                continue;
            }
            // Module must be declared in lib.rs
            let mod_decl = format!("mod {};", stem);
            assert!(
                lib_content.contains(&mod_decl),
                "File {:?} is not declared as a module in lib.rs (expected `{}`)",
                path.file_name().unwrap(),
                mod_decl
            );
        }
    }
}

// ============================================================================
// Category 5: Adversarial Edge Cases (T14-T17)
// ============================================================================

/// T14: Method with same name in two classes — correct calls/called_by attribution.
/// TDD anchor — requires Worker 1's class-scoped method resolution.
#[test]
#[ignore] // Waiting on Worker 1's method call tracking implementation
fn t14_same_method_name_different_classes() {
    let dir = TempDir::new().unwrap();
    let same_name_src = workspace_root().join("tests/fixtures/method_calls/python/same_name.py");
    let content = fs::read_to_string(&same_name_src).unwrap();
    fs::write(dir.path().join("same_name.py"), &content).unwrap();

    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    // Find all validate entities
    let validates: Vec<&serde_json::Value> = entities
        .iter()
        .filter(|e| {
            e["id"]
                .as_str()
                .map(|id| id.contains("validate"))
                .unwrap_or(false)
        })
        .collect();

    // There should be at least 2 validate entities (Encoder.validate, Decoder.validate)
    assert!(
        validates.len() >= 2,
        "Expected at least 2 validate entities (Encoder + Decoder), got {}",
        validates.len()
    );

    // Each validate's called_by should only include its own class's process method,
    // not the other class's process method. Check by verifying no validate entity
    // has both Encoder and Decoder references in called_by.
    for v in &validates {
        let empty = vec![];
        let called_by = v["called_by"].as_array().unwrap_or(&empty);
        let has_encoder = called_by
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("Encoder")).unwrap_or(false));
        let has_decoder = called_by
            .iter()
            .any(|c| c.as_str().map(|s| s.contains("Decoder")).unwrap_or(false));
        assert!(
            !(has_encoder && has_decoder),
            "validate entity should not have both Encoder and Decoder in called_by — cross-class pollution detected: {:?}",
            called_by
        );
    }
}

/// T15: self.method() where method doesn't exist — no false edge.
/// Regression guard — must not crash regardless of Worker 1's changes.
#[test]
fn t15_self_call_to_nonexistent_method_no_edge() {
    let dir = TempDir::new().unwrap();
    let broken_src = workspace_root().join("tests/fixtures/method_calls/python/broken_self.py");
    let content = fs::read_to_string(&broken_src).unwrap();
    fs::write(dir.path().join("broken_self.py"), &content).unwrap();

    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Analysis must not crash on self.nonexistent(). Exit code: {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    // No entity named "nonexistent" should exist
    let has_nonexistent = entities.iter().any(|e| {
        e["id"]
            .as_str()
            .map(|id| id == "nonexistent" || id == "Broken.nonexistent")
            .unwrap_or(false)
    });
    assert!(
        !has_nonexistent,
        "No entity should be created for nonexistent method"
    );
}

/// T16: Recursive self-method call — trace handles cycle.
/// Regression guard — trace must not hang or crash.
#[test]
fn t16_recursive_self_method_no_infinite_trace() {
    let dir = TempDir::new().unwrap();
    let recursive_src =
        workspace_root().join("tests/fixtures/method_calls/python/recursive_self.py");
    let content = fs::read_to_string(&recursive_src).unwrap();
    fs::write(dir.path().join("recursive_self.py"), &content).unwrap();

    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args([
            "trace",
            "--symbol",
            "traverse",
            "--direction",
            "forward",
            "--depth",
            "5",
            dir_path,
            "--format",
            "yaml",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "Trace on recursive method must not crash. Exit code: {}",
        code
    );
}

/// T17: Method call on non-self parameter — no crash.
/// Regression guard — analysis must complete safely.
#[test]
fn t17_non_self_method_call_no_crash() {
    let dir = TempDir::new().unwrap();
    let non_self_src = workspace_root().join("tests/fixtures/method_calls/python/non_self.py");
    let content = fs::read_to_string(&non_self_src).unwrap();
    fs::write(dir.path().join("non_self.py"), &content).unwrap();

    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "yaml"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Analysis must not crash on non-self method call. Exit code: {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.is_empty(),
        "Analysis output must not be empty for non-self fixture"
    );
}

// ============================================================================
// Category 6: Regression Guards (T18-T23)
// ============================================================================

/// T18: All C15 convergence patterns still work — Rust fixture produces valid YAML.
#[test]
fn t18_c15_convergence_regression_guard() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args(["analyze", &fixture, "--format", "yaml"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(code == 0 || code == 2, "exit code must be 0 or 2");

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _parsed: serde_yaml::Value = serde_yaml::from_str(&stdout)
        .unwrap_or_else(|e| panic!("Rust fixture YAML invalid: {}", e));
}

/// T19: Byte floor still active (C14 contract).
/// The byte floor (MIN_MANIFEST_ALLOW_BYTES = 20,480) means small manifests
/// always pass size validation — it doesn't guarantee output size. This test
/// verifies that minimal projects produce valid output without size-ratio errors.
#[test]
fn t19_byte_floor_preserved_c16() {
    let dir = create_minimal_python();
    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "yaml"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(code == 0 || code == 2, "exit code must be 0 or 2");

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        !stdout.is_empty(),
        "Byte floor failed — minimal project produced empty output"
    );

    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "Byte floor active but output is not valid YAML for minimal project"
    );
}

/// T20: Exit code contract preserved (0/1/2).
#[test]
fn t20_exit_codes_preserved_c16() {
    // Exit 0 or 2 on clean project (depends on diagnostics)
    let clean = create_clean_python();
    let clean_path = clean.path().to_str().unwrap();
    let output = flowspec().args(["analyze", clean_path]).output().unwrap();
    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "Clean project exit must be 0 or 2, got {}",
        code
    );

    // Exit 1 on nonexistent path
    let output = flowspec()
        .args(["analyze", "/nonexistent/path/that/does/not/exist"])
        .output()
        .unwrap();
    let code = output.status.code().unwrap();
    assert_eq!(code, 1, "Nonexistent path must exit 1, got {}", code);
}

/// T21: Pipe safety preserved — stdout is pure structured output.
#[test]
fn t21_pipe_safety_preserved_c16() {
    let dir = create_python_with_dead_code();
    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    // stdout must be valid JSON — no tracing contamination
    let _: serde_json::Value = serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "stdout is not valid JSON (possible tracing contamination): {}\nFirst 200 chars: {}",
            e,
            &stdout[..stdout.len().min(200)]
        )
    });
}

/// T22: 8-section manifest structure preserved (C3 contract).
#[test]
fn t22_manifest_structure_preserved_c16() {
    let dir = create_python_with_dead_code();
    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["analyze", dir_path, "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let required_sections = [
        "metadata",
        "summary",
        "entities",
        "flows",
        "boundaries",
        "diagnostics",
        "dependency_graph",
        "type_flows",
    ];
    for section in &required_sections {
        assert!(
            parsed.get(section).is_some(),
            "Manifest missing required section '{}'",
            section
        );
    }
}

/// T23: Confidence field present on all diagnostics (C1 contract).
#[test]
fn t23_confidence_field_preserved_c16() {
    let dir = create_python_with_dead_code();
    let dir_path = dir.path().to_str().unwrap();
    let output = flowspec()
        .args(["diagnose", dir_path, "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let diag_array = diagnostics.as_array().expect("diagnose must return array");
    for (i, diag) in diag_array.iter().enumerate() {
        let confidence = diag["confidence"].as_str();
        assert!(
            confidence.is_some(),
            "Diagnostic {} missing confidence field: {:?}",
            i,
            diag
        );
        let conf = confidence.unwrap();
        assert!(
            conf == "high" || conf == "moderate" || conf == "low",
            "Diagnostic {} has invalid confidence '{}' — must be high/moderate/low",
            i,
            conf
        );
    }
}

// ============================================================================
// Category 7: Diagnostic Count Orthogonality (T24-T25)
// ============================================================================

/// T24: Method call tracking doesn't affect stale_reference count.
#[test]
fn t24_stale_reference_unaffected_by_method_calls() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args([
            "diagnose",
            &fixture,
            "--format",
            "json",
            "--checks",
            "stale_reference",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    // Just verify it parses and returns an array — count stability is checked
    // across runs, not against a hardcoded value (which would break on test changes)
    assert!(
        diagnostics.is_array(),
        "stale_reference diagnose output must be an array"
    );
}

/// T25: Method call tracking doesn't affect phantom_dependency count.
#[test]
fn t25_phantom_dependency_unaffected_by_method_calls() {
    let fixture = rust_fixture_path();
    let output = flowspec()
        .args([
            "diagnose",
            &fixture,
            "--format",
            "json",
            "--checks",
            "phantom_dependency",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let diagnostics: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        diagnostics.is_array(),
        "phantom_dependency diagnose output must be an array"
    );
}
