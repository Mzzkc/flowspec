use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

// === Category 1: JS Analysis Output Validity (P0) ===

#[test]
fn js_yaml_output_is_valid_yaml() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("hello.js"),
        "function hello() { return 1; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "JS analysis exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "JS analysis stdout is not valid YAML: {:?}\nFirst 500 chars: {}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn js_json_output_is_valid_json() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("hello.js"),
        "function hello() { return 1; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "JS JSON analysis exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: Result<serde_json::Value, _> = serde_json::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "JS analysis stdout is not valid JSON: {:?}\nFirst 500 chars: {}",
        parsed.err(),
        &stdout[..stdout.len().min(500)]
    );
}

#[test]
fn js_manifest_has_all_eight_sections() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.js"), "function main() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("Not valid JSON: {}", e));

    let obj = parsed.as_object().expect("JSON root must be an object");
    let required = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    for section in &required {
        assert!(
            obj.contains_key(*section),
            "Missing required section '{}' in JS manifest. Keys: {:?}",
            section,
            obj.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn js_exit_codes_only_0_1_2() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("valid.js"), "function f() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 1 || code == 2,
        "JS analysis returned exit code {}. Only 0, 1, 2 are valid.",
        code
    );
}

// === Category 2: Multi-Language Manifest Coherence (P0) ===

#[test]
fn mixed_dir_metadata_lists_both_languages() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("hello.py"), "def hello(): pass\n").unwrap();
    fs::write(dir.path().join("app.js"), "function app() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let langs: Vec<String> = parsed["metadata"]["languages"]
        .as_array()
        .expect("languages must be array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    assert!(
        langs.contains(&"python".to_string()),
        "languages {:?} missing 'python'",
        langs
    );
    assert!(
        langs.contains(&"javascript".to_string()),
        "languages {:?} missing 'javascript'",
        langs
    );
}

#[test]
fn mixed_dir_entities_from_both_languages() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("hello.py"), "def hello():\n    return 1\n").unwrap();
    fs::write(dir.path().join("app.js"), "function app() { return 2; }\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");

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
        has_py,
        "No Python entities found in mixed-language analysis"
    );
    assert!(
        has_js,
        "No JavaScript entities found in mixed-language analysis"
    );
}

#[test]
fn js_only_analysis_has_entities() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("funcs.js"),
        "function greet(name) { return 'Hi ' + name; }\nfunction main() { greet('world'); }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");

    assert!(
        !entities.is_empty(),
        "JS analysis produced 0 entities. Hard gate: >= 1 entity required."
    );
}

#[test]
fn js_json_yaml_same_section_keys() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("lib.js"), "function helper() {}\n").unwrap();

    let yaml_out = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    let yaml_val: serde_yaml::Value =
        serde_yaml::from_str(&String::from_utf8(yaml_out.stdout).unwrap()).unwrap();

    let json_out = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();
    let json_val: serde_json::Value =
        serde_json::from_str(&String::from_utf8(json_out.stdout).unwrap()).unwrap();

    let mut yaml_keys: Vec<String> = yaml_val
        .as_mapping()
        .unwrap()
        .keys()
        .map(|k| k.as_str().unwrap().to_string())
        .collect();
    let mut json_keys: Vec<String> = json_val.as_object().unwrap().keys().cloned().collect();
    yaml_keys.sort();
    json_keys.sort();

    assert_eq!(
        yaml_keys, json_keys,
        "JS analysis: YAML sections {:?} != JSON sections {:?}",
        yaml_keys, json_keys
    );
}

// === Category 3: Pipe Safety and CLI Flags for JS (P1) ===

#[test]
fn js_stdout_pipe_safe() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.js"), "const x = () => 42;\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("JS stdout is not valid YAML — pipe safety violated");

    let log_patterns = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    for line in stdout.lines() {
        let trimmed = line.trim();
        for pat in &log_patterns {
            assert!(
                !trimmed.starts_with(pat) && !trimmed.starts_with(&format!("[{}]", pat)),
                "JS stdout contains log line: '{}'. Logs must go to stderr only.",
                line
            );
        }
    }
}

#[test]
fn js_output_flag_writes_to_file() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("lib.js"), "function lib() {}\n").unwrap();
    let outfile = tempfile::NamedTempFile::new().unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "analyze",
            dir.path().to_str().unwrap(),
            "--format",
            "json",
            "--output",
            outfile.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "stdout should be empty with --output. Got: {}",
        &stdout[..stdout.len().min(200)]
    );

    let content = fs::read_to_string(outfile.path()).unwrap();
    assert!(
        !content.is_empty(),
        "--output file is empty for JS analysis"
    );
    let _: serde_json::Value =
        serde_json::from_str(&content).expect("--output file is not valid JSON for JS analysis");
}

#[test]
fn js_quiet_flag_no_stderr() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.js"), "function f() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--quiet"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.is_empty(),
        "JS --quiet should produce no stderr. Got:\n{}",
        stderr
    );
}

// === Category 4: Adversarial JS Edge Cases (P1) ===

#[test]
fn empty_js_file_valid_output() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("empty.js"), "").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(0),
        "Empty JS file should exit 0 (no findings), got {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("Empty JS file output is not valid JSON");

    let entities = parsed["entities"]
        .as_array()
        .expect("entities must be array");
    assert!(
        entities.is_empty(),
        "Empty JS file should produce 0 entities, got {}",
        entities.len()
    );
}

#[test]
fn js_only_comments_valid_output() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("comments.js"),
        "// This is a comment\n/* Block comment\n   spanning lines */\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(0));

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entities = parsed["entities"].as_array().unwrap();
    assert!(
        entities.is_empty(),
        "Comments-only JS should produce 0 entities, got {}",
        entities.len()
    );
}

#[test]
fn js_syntax_errors_no_panic() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("broken.js"),
        "function { } } } const = ;\nlet 123abc = true;\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap()])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "JS syntax error should not cause panic (exit 101) or error (exit 1). Got: {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let _: serde_yaml::Value =
        serde_yaml::from_str(&stdout).expect("JS syntax error output is not valid YAML");
}

#[test]
fn ts_tsx_extensions_recognized() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("app.ts"),
        "function greet(): string { return 'hi'; }\n",
    )
    .unwrap();
    fs::write(
        dir.path().join("comp.tsx"),
        "function App() { return null; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "TS/TSX analysis exit code must be 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();

    let langs: Vec<String> = parsed["metadata"]["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        langs.contains(&"typescript".to_string()),
        "languages {:?} should include 'typescript' for .ts/.tsx files",
        langs
    );
}

#[test]
fn jsx_recognized_as_javascript() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("comp.jsx"),
        "function Component() { return null; }\n",
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", dir.path().to_str().unwrap(), "--format", "json"])
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
        langs.contains(&"javascript".to_string()),
        ".jsx should be recognized as JavaScript. Got: {:?}",
        langs
    );
}

// === Category 5: Regression Guards (P2) ===

#[test]
fn python_analysis_unchanged_with_js_adapter() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("code.py"),
        r#"
def used():
    return 42

def dead():
    return "never called"

def main():
    result = used()
    print(result)
"#,
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

    assert!(
        entities.len() >= 3,
        "Python analysis should produce >= 3 entities (used, dead, main). Got {}",
        entities.len()
    );

    let langs: Vec<String> = parsed["metadata"]["languages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    assert!(
        langs.contains(&"python".to_string()),
        "Python-only dir should list python in languages: {:?}",
        langs
    );
}

#[test]
fn python_exit_codes_unchanged_by_js_adapter() {
    let clean = TempDir::new().unwrap();
    fs::write(
        clean.path().join("clean.py"),
        "def greet(name):\n    return f'Hello, {name}'\n\ndef main():\n    greet('world')\n",
    )
    .unwrap();

    Command::cargo_bin("flowspec")
        .unwrap()
        .args(["analyze", clean.path().to_str().unwrap()])
        .assert()
        .code(0);

    let issues = TempDir::new().unwrap();
    fs::write(
        issues.path().join("dead.py"),
        "import os\ndef used(): return 42\ndef dead(): return 0\ndef main(): used()\n",
    )
    .unwrap();

    Command::cargo_bin("flowspec")
        .unwrap()
        .args(["diagnose", issues.path().to_str().unwrap()])
        .assert()
        .code(2);
}

// === Category 6: Diagnose Command with JS (P1) ===

#[test]
fn js_dead_code_diagnose_exit_2() {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("dead.js"),
        r#"
function used() { return 42; }
function unreachable() { return "never called"; }
function main() { console.log(used()); }
"#,
    )
    .unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["diagnose", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    assert_eq!(
        output.status.code(),
        Some(2),
        "JS dead code should produce findings (exit 2). Got: {:?}",
        output.status.code()
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(
        parsed.as_array().unwrap().len() > 0,
        "diagnose should produce >= 1 diagnostic for JS dead code"
    );
}

#[test]
fn js_diagnose_json_valid_array() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("code.js"), "function main() {}\n").unwrap();

    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args(["diagnose", dir.path().to_str().unwrap(), "--format", "json"])
        .output()
        .unwrap();

    let code = output.status.code().unwrap();
    assert!(
        code == 0 || code == 2,
        "diagnose must exit 0 or 2, got {}",
        code
    );

    let stdout = String::from_utf8(output.stdout).unwrap();
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("JS diagnose --format json is not valid JSON");
    assert!(
        parsed.is_array(),
        "JS diagnose output must be a JSON array, got: {:?}",
        parsed
    );
}
