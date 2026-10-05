//! Public report state and selected-input loss, exercised through the shipped CLI.

use flowspec::{analyze, Config};
use serde_json::{json, Value};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

const UNUSED: &str = "def _unused():\n    return 1\n";
const PYTHON_ERROR: &str = "def broken(\n";
const JAVASCRIPT_MISSING: &str = "function f() {\n";

fn project(files: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new().expect("temporary project");
    for (name, content) in files {
        fs::write(dir.path().join(name), content).expect("write fixture");
    }
    dir
}

fn cli(path: &Path, format: &str, options: &[&str]) -> (i32, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(path)
        .args(["--format", format])
        .args(options)
        .output()
        .expect("run flowspec analyze");
    let stdout = String::from_utf8(output.stdout).expect("UTF-8 report");
    let stderr = String::from_utf8(output.stderr).expect("UTF-8 stderr");
    assert!(!stdout.is_empty(), "empty report; stderr: {stderr}");
    (output.status.code().expect("normal process exit"), stdout)
}

fn json_report(path: &Path, options: &[&str]) -> (i32, Value) {
    let (code, output) = cli(path, "json", options);
    let report = serde_json::from_str(&output).expect("valid JSON report");
    (code, report)
}

fn state(report: &Value, execution: &str, findings: bool, outcome: &str) {
    assert_eq!(report["schema_version"], json!(2), "schema v2: {report}");
    assert_eq!(report["execution"], execution, "execution: {report}");
    assert_eq!(report["findings_present"], findings, "findings: {report}");
    assert_eq!(report["outcome"], outcome, "outcome: {report}");
    for section in [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ] {
        assert!(report.get(section).is_some(), "missing {section}: {report}");
    }
}

fn counts(report: &Value, selected: u64, read: u64, parsed: u64, rejected: u64) {
    let ledger = &report["completeness"];
    assert_eq!(ledger["selected"], selected, "selected: {ledger}");
    assert_eq!(ledger["read"], read, "read: {ledger}");
    assert_eq!(ledger["parsed"], parsed, "parsed: {ledger}");
    assert_eq!(ledger["rejected"], rejected, "rejected: {ledger}");
    assert_eq!(selected, parsed + rejected, "fixture accounting");
}

fn rejection(report: &Value, name: &str, reason: &str) {
    let files = report["completeness"]["rejected_files"]
        .as_array()
        .expect("structured per-file rejections");
    assert!(
        files.iter().any(|file| {
            file["path"]
                .as_str()
                .is_some_and(|path| path.ends_with(name))
                && file["reason"] == reason
        }),
        "missing {reason} for {name}: {files:?}"
    );
}

fn positive(report: &Value) -> Value {
    let diagnostic = report["diagnostics"]
        .as_array()
        .expect("diagnostics array")
        .iter()
        .find(|item| {
            item["pattern"] == "data_dead_end"
                && item["entity"] == "unused.py::_unused"
                && item["loc"] == "unused.py:1"
        })
        .expect("valid positive finding for unused.py::_unused at unused.py:1");
    json!({
        "pattern": diagnostic["pattern"],
        "entity": diagnostic["entity"],
        "loc": diagnostic["loc"],
    })
}

#[test]
fn complete_clean_report_v2() {
    let dir = project(&[("clean.py", b"pass\n")]);
    let (code, report) = json_report(dir.path(), &[]);
    assert_eq!(code, 0);
    state(&report, "complete", false, "clean");
    counts(&report, 1, 1, 1, 0);
    assert!(report["diagnostics"].as_array().unwrap().is_empty());
    assert!(report["completeness"]["rejected_files"]
        .as_array()
        .unwrap()
        .is_empty());
}

#[test]
fn complete_findings_report_v2() {
    let dir = project(&[(
        "bad.py",
        b"class Box:\n    @staticmethod\n    def open(self):\n        return 1\n",
    )]);
    let (code, report) = json_report(dir.path(), &[]);
    assert_eq!(
        code, 3,
        "raw legacy critical observation is unsupported authority"
    );
    state(&report, "incomplete", false, "incomplete");
    counts(&report, 1, 1, 1, 0);
    assert_eq!(report["authority"]["gate"]["verdict"], "not_evaluated");
    assert_eq!(report["authority"]["findings"]["total"], 0);
    assert!(report["authority"]["losses"]
        .as_array()
        .unwrap()
        .iter()
        .any(|loss| loss["reason"] == "semantic_proof_unavailable"));
    assert!(report["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["pattern"] == "contract_mismatch"
                && item["entity"] == "open"
                && item["loc"] == "bad.py:3"
                && item["severity"] == "critical"
        }));
}

#[test]
fn unreadable_selected_input_is_incomplete() {
    let dir = project(&[("unreadable.py", b"pass\n")]);
    let path = dir.path().join("unreadable.py");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();
    assert!(
        fs::read(&path).is_err(),
        "the test process must actually observe a read failure"
    );
    let (code, report) = json_report(dir.path(), &[]);
    assert_eq!(code, 3, "selected unreadable file must not look clean");
    state(&report, "incomplete", false, "incomplete");
    counts(&report, 1, 0, 0, 1);
    rejection(&report, "unreadable.py", "read_error");
}

#[test]
fn malformed_selected_input_is_incomplete() {
    // The Python fixture produces an ERROR node; the JavaScript fixture
    // produces a missing recovery node without an ERROR node.
    let dir = project(&[
        ("error.py", PYTHON_ERROR.as_bytes()),
        ("missing.js", JAVASCRIPT_MISSING.as_bytes()),
    ]);
    let (code, report) = json_report(dir.path(), &[]);
    assert_eq!(code, 3, "tree-sitter loss must determine the exit");
    state(&report, "incomplete", false, "incomplete");
    counts(&report, 2, 2, 0, 2);
    rejection(&report, "error.py", "parse_error");
    rejection(&report, "missing.js", "parse_error");
}

#[test]
fn incomplete_retains_positive_findings() {
    let dir = project(&[("unused.py", UNUSED.as_bytes())]);
    let (before_code, before) = json_report(dir.path(), &[]);
    assert_eq!(before_code, 0, "warning does not meet critical exit gate");
    state(&before, "complete", true, "findings");
    counts(&before, 1, 1, 1, 0);
    let expected = positive(&before);

    fs::write(dir.path().join("missing.js"), JAVASCRIPT_MISSING).unwrap();
    let (after_code, after) = json_report(dir.path(), &[]);
    assert_eq!(after_code, 3);
    state(&after, "incomplete", true, "incomplete");
    counts(&after, 2, 2, 1, 1);
    rejection(&after, "missing.js", "parse_error");
    assert_eq!(
        positive(&after),
        expected,
        "positive subject/pattern/range changed"
    );
}

#[test]
fn filtered_incomplete_still_exits_three() {
    let dir = project(&[
        ("unused.py", UNUSED.as_bytes()),
        ("missing.js", JAVASCRIPT_MISSING.as_bytes()),
    ]);
    let (code, report) = json_report(
        dir.path(),
        &[
            "--checks",
            "contract_mismatch",
            "--severity",
            "critical",
            "--confidence",
            "high",
        ],
    );
    assert_eq!(code, 3, "display filtering cannot mask input loss");
    state(&report, "incomplete", true, "incomplete");
    counts(&report, 2, 2, 1, 1);
    assert!(report["diagnostics"].as_array().unwrap().is_empty());
    rejection(&report, "missing.js", "parse_error");
}

#[test]
fn yaml_and_summary_expose_incomplete() {
    let dir = project(&[("missing.js", JAVASCRIPT_MISSING.as_bytes())]);
    let (yaml_code, yaml) = cli(dir.path(), "yaml", &[]);
    assert_eq!(yaml_code, 3);
    let yaml: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid YAML report");
    assert_eq!(yaml["schema_version"].as_u64(), Some(2));
    assert_eq!(yaml["execution"].as_str(), Some("incomplete"));
    assert_eq!(yaml["findings_present"].as_bool(), Some(false));
    assert_eq!(yaml["outcome"].as_str(), Some("incomplete"));
    assert_eq!(yaml["completeness"]["selected"].as_u64(), Some(1));
    assert_eq!(yaml["completeness"]["read"].as_u64(), Some(1));
    assert_eq!(yaml["completeness"]["parsed"].as_u64(), Some(0));
    assert_eq!(yaml["completeness"]["rejected"].as_u64(), Some(1));
    assert_eq!(
        yaml["completeness"]["rejected_files"][0]["reason"].as_str(),
        Some("parse_error")
    );

    let (summary_code, summary) = cli(dir.path(), "summary", &[]);
    assert_eq!(summary_code, 3);
    for required in [
        "schema_version: 2",
        "execution: incomplete",
        "findings_present: false",
        "outcome: incomplete",
        "selected: 1",
        "read: 1",
        "parsed: 0",
        "rejected: 1",
        "parse_error",
    ] {
        assert!(
            summary.contains(required),
            "summary missing {required}: {summary}"
        );
    }
}

#[test]
fn library_and_cli_agree_on_execution() {
    let dir = project(&[("clean.py", b"pass\n"), ("invalid.py", &[0xff, 0xfe])]);
    let library = analyze(dir.path(), &Config::default(), &[]).expect("library report");
    let library = serde_json::to_value(&library.manifest).expect("serializable public report");
    let (code, cli_report) = json_report(dir.path(), &[]);
    assert_eq!(code, 3, "invalid UTF-8 is selected read loss");
    state(&library, "incomplete", false, "incomplete");
    counts(&library, 2, 1, 1, 1);
    rejection(&library, "invalid.py", "read_error");
    for field in [
        "schema_version",
        "execution",
        "findings_present",
        "outcome",
        "completeness",
    ] {
        assert_eq!(
            library[field], cli_report[field],
            "library/CLI {field} drift"
        );
    }
}
