use assert_cmd::Command;
use std::fs;
use tempfile::TempDir;

fn create_clean_project() -> TempDir {
    let dir = TempDir::new().unwrap();
    fs::write(
        dir.path().join("clean.py"),
        r#"
def greet(name: str) -> str:
    return f"Hello, {name}"

def main():
    result = greet("world")
    print(result)

if __name__ == "__main__":
    main()
"#,
    )
    .unwrap();
    dir
}

#[test]
fn stdout_contains_only_yaml() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();

    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&stdout);
    assert!(
        parsed.is_ok(),
        "stdout contains non-YAML content. Pipe safety violated.\nFirst 200 chars: {}",
        &stdout[..stdout.len().min(200)]
    );

    let log_patterns = ["TRACE", "DEBUG", "INFO", "WARN", "ERROR"];
    for pattern in &log_patterns {
        for line in stdout.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with(pattern) || trimmed.starts_with(&format!("[{}]", pattern)) {
                panic!(
                    "stdout contains log-like line: '{}'. Logs must go to stderr.",
                    line
                );
            }
        }
    }
}

#[test]
fn stderr_contains_logs_when_verbose() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap(), "--verbose"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        !stderr.is_empty(),
        "--verbose produced no stderr output. Expected tracing logs."
    );
}

#[test]
fn stderr_empty_when_quiet_and_no_errors() {
    let project = create_clean_project();
    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args(["analyze", project.path().to_str().unwrap(), "--quiet"])
        .output()
        .unwrap();

    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(
        stderr.is_empty(),
        "--quiet should produce no stderr output on success. Got:\n{}",
        stderr
    );
}

#[test]
fn output_flag_writes_to_file_not_stdout() {
    let project = create_clean_project();
    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap();

    let mut cmd = Command::cargo_bin("flowspec").unwrap();
    let output = cmd
        .args([
            "analyze",
            project.path().to_str().unwrap(),
            "--output",
            output_path,
        ])
        .output()
        .unwrap();

    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.trim().is_empty(),
        "stdout should be empty when --output is used. Got: {}",
        &stdout[..stdout.len().min(200)]
    );

    let file_content = std::fs::read_to_string(output_path).unwrap();
    assert!(!file_content.is_empty(), "--output file is empty");
    let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&file_content);
    assert!(
        parsed.is_ok(),
        "--output file is not valid YAML: {:?}",
        parsed.err()
    );
}

#[test]
fn output_file_matches_stdout_content() {
    let project = create_clean_project();

    let mut cmd1 = Command::cargo_bin("flowspec").unwrap();
    let stdout_output = cmd1
        .args(["analyze", project.path().to_str().unwrap()])
        .output()
        .unwrap();
    let stdout_content = String::from_utf8(stdout_output.stdout).unwrap();

    let output_file = tempfile::NamedTempFile::new().unwrap();
    let output_path = output_file.path().to_str().unwrap();
    let mut cmd2 = Command::cargo_bin("flowspec").unwrap();
    cmd2.args([
        "analyze",
        project.path().to_str().unwrap(),
        "--output",
        output_path,
    ])
    .assert()
    .success();

    let file_content = std::fs::read_to_string(output_path).unwrap();

    let stdout_yaml: serde_yaml::Value = serde_yaml::from_str(&stdout_content).unwrap();
    let file_yaml: serde_yaml::Value = serde_yaml::from_str(&file_content).unwrap();

    let stdout_keys: Vec<_> = stdout_yaml.as_mapping().unwrap().keys().collect();
    let file_keys: Vec<_> = file_yaml.as_mapping().unwrap().keys().collect();
    assert_eq!(
        stdout_keys, file_keys,
        "stdout and --output file have different manifest sections"
    );
}
