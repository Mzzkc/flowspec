use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

// === Category 1: --language Flag Filtering (P1, Highest Value) ===

#[test]
fn language_python_filters_to_python_only() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def greet():\n    return 'hi'\n").unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function greet() { return 'hi'; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    let has_py = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".py"))
            .unwrap_or(false)
    });
    assert!(
        has_py,
        "No Python entities found when --language python specified"
    );

    let has_js = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".js"))
            .unwrap_or(false)
    });
    assert!(
        !has_js,
        "JS entities leaked through when --language python was set. Found: {:?}",
        entities
            .iter()
            .filter(|e| e["loc"]
                .as_str()
                .map(|l| l.contains(".js"))
                .unwrap_or(false))
            .collect::<Vec<_>>()
    );
}

#[test]
fn language_javascript_filters_to_js_only() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def greet():\n    return 'hi'\n").unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function greet() { return 'hi'; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "javascript",
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    let has_js = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".js"))
            .unwrap_or(false)
    });
    assert!(
        has_js,
        "No JS entities found when --language javascript specified"
    );

    let has_py = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".py"))
            .unwrap_or(false)
    });
    assert!(
        !has_py,
        "Python entities leaked through when --language javascript was set"
    );
}

#[test]
fn language_rust_produces_empty_results_not_silent() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def main(): pass\n").unwrap();
    fs::write(dir.path().join("app.js"), "function main() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "rust",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_ne!(
        code, 2,
        "--language rust should not produce exit 2 (findings). No rust adapter exists."
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    assert!(
        entities.is_empty(),
        "--language rust produced {} entities. No Rust adapter exists, so zero expected. IDs: {:?}",
        entities.len(),
        entities
            .iter()
            .map(|e| e["id"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.contains("panic"),
        "Rust language request caused a panic"
    );
}

#[test]
fn no_language_flag_analyzes_all_languages() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("hello.py"), "def hello(): return 1\n").unwrap();
    fs::write(
        dir.path().join("world.js"),
        "function world() { return 2; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    let has_py = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".py"))
            .unwrap_or(false)
    });
    let has_js = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".js"))
            .unwrap_or(false)
    });

    assert!(
        has_py && has_js,
        "Without --language flag, both Python and JS must be analyzed. py={}, js={}, entities={:?}",
        has_py,
        has_js,
        entities
            .iter()
            .map(|e| e["loc"].as_str().unwrap_or("?"))
            .collect::<Vec<_>>()
    );
}

#[test]
fn language_typescript_activates_js_adapter_for_ts_files() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.ts"),
        "function greet(): string { return 'hi'; }\n",
    )
    .unwrap();
    fs::write(dir.path().join("util.py"), "def helper(): return 42\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "typescript",
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
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    let has_ts = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".ts"))
            .unwrap_or(false)
    });
    assert!(
        has_ts,
        "--language typescript on .ts files should produce entities via JS adapter"
    );

    let has_py = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".py"))
            .unwrap_or(false)
    });
    assert!(
        !has_py,
        "Python entities leaked through when --language typescript was set"
    );
}

#[test]
fn multiple_language_flags_enable_both() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def py_fn(): return 1\n").unwrap();
    fs::write(
        dir.path().join("app.js"),
        "function js_fn() { return 2; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
            "--language",
            "javascript",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    let has_py = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".py"))
            .unwrap_or(false)
    });
    let has_js = entities.iter().any(|e| {
        e["loc"]
            .as_str()
            .map(|l| l.contains(".js"))
            .unwrap_or(false)
    });

    assert!(
        has_py && has_js,
        "--language python --language javascript should analyze both. py={}, js={}",
        has_py,
        has_js
    );
}

#[test]
fn language_invalid_exits_1() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def f(): pass\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--language",
            "cobol",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(1),
        "Invalid language name should exit 1. Got: {:?}",
        output.status.code()
    );

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.contains("cobol") || stderr.contains("unsupported"),
        "Error message should mention the invalid language. stderr: {}",
        stderr
    );
}

#[test]
fn language_flag_sets_metadata_languages() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def f(): pass\n").unwrap();
    fs::write(dir.path().join("app.js"), "function f() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let langs: Vec<String> = parsed["metadata"]["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert!(
        langs.contains(&"python".to_string()),
        "metadata.languages should contain 'python' when --language python set. Got: {:?}",
        langs
    );

    assert!(
        !langs.contains(&"javascript".to_string()),
        "metadata.languages should NOT contain 'javascript' when --language python. Got: {:?}",
        langs
    );
}

#[test]
fn language_python_on_js_only_dir_produces_zero_entities() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.js"), "function main() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 0,
        "No Python files to analyze, should exit 0 (clean). Got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();

    assert!(
        entities.is_empty(),
        "--language python on JS-only dir should produce 0 entities, got {}",
        entities.len()
    );
}

// === Category 2: Un-Ignored Test Verification (P0) ===

#[test]
fn javascript_output_no_stale_ignores() {
    let content = fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/javascript_output.rs"
    ))
    .unwrap();

    let ignore_count = content.matches("#[ignore").count();
    assert_eq!(
        ignore_count, 0,
        "javascript_output.rs still has {} #[ignore] annotations. All should be removed.",
        ignore_count
    );
}

// === Category 3: SUPPORTED_LANGUAGES Warning (P2) ===

#[test]
fn language_rust_emits_warning_on_stderr() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("main.rs"), "fn main() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "rust",
            "--verbose",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let has_warning = stderr.to_lowercase().contains("warn")
        && (stderr.contains("rust")
            || stderr.contains("no analysis")
            || stderr.contains("no entities"));
    assert!(
        has_warning,
        "--language rust --verbose should emit warning about no working adapter. stderr:\n{}",
        stderr
    );
}

#[test]
fn language_python_no_spurious_warning() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def main(): pass\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
            "--verbose",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let has_no_results_warning = stderr.to_lowercase().contains("no analysis results")
        || stderr.to_lowercase().contains("no entities");
    assert!(
        !has_no_results_warning,
        "Valid Python analysis with --language python should not warn about missing results. stderr:\n{}",
        stderr
    );
}

#[test]
fn language_javascript_no_spurious_warning() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.js"), "function main() { return 1; }\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "javascript",
            "--verbose",
        ])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    let has_no_results_warning = stderr.to_lowercase().contains("no analysis results")
        || stderr.to_lowercase().contains("no entities");
    assert!(
        !has_no_results_warning,
        "Valid JS analysis should not warn about missing results. stderr:\n{}",
        stderr
    );
}

// === Category 4: Exit Code Contract Preservation (P1) ===

#[test]
fn language_filter_preserves_exit_2_for_findings() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead.py"),
        "import os\ndef used(): return 42\ndef dead(): return 0\ndef main(): used()\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["diagnose", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert_eq!(
        code, 2,
        "Python with dead code + phantom dep should exit 2 from diagnose. Got {}",
        code
    );
}

#[test]
fn language_filter_preserves_exit_0_for_clean() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        "def greet(name):\n    return f'Hello, {name}'\n\ndef main():\n    greet('world')\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "Clean Python analysis with --language python should exit 0"
    );
}

// === Category 5: Pipe Safety Under Language Filtering (P2) ===

#[test]
fn language_filtered_output_is_pipe_safe() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "def f(): pass\n").unwrap();
    fs::write(dir.path().join("app.js"), "function f() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--language",
            "python",
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "Pipe safety violated: --language filtered output is not valid JSON.\nFirst 300 chars: {}",
        &stdout[..stdout.len().min(300)]
    );

    for line in stdout.lines() {
        let trimmed = line.trim();
        for pat in &["TRACE", "DEBUG", "INFO", "WARN", "ERROR"] {
            assert!(
                !trimmed.starts_with(pat),
                "Log line leaked to stdout: '{}'",
                line
            );
        }
    }
}
