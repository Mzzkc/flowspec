//! Contract RED: inspect the real analyze CLI's JSON, without future Rust report types.
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::os::unix::ffi::OsStringExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

const HELPER: &[u8] = b"def _unused_helper():\n    return 1\n";
const MAIN: &[u8] = b"def main():\n    return 1\n\nif __name__ == \"__main__\":\n    main()\n";

fn project(files: &[(&str, &[u8])]) -> TempDir {
    let dir = TempDir::new().expect("temporary project");
    for (name, bytes) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("parent")).expect("create fixture parent");
        fs::write(path, bytes).expect("write exact fixture bytes");
    }
    dir
}

fn run(root: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(root)
        .args(["--format", "json"])
        .args(args)
        .output()
        .expect("run packaged analyze CLI")
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "analyze must emit JSON: {error}; exit={:?}; stderr={}; stdout={}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        )
    })
}

fn authority(report: &Value) -> &Value {
    report
        .get("authority")
        .expect("analyzer-authoritative JSON report")
}

fn private_finding(report: &Value) -> &Value {
    let findings = authority(report)["findings"]["all"]
        .as_array()
        .expect("full findings, independent of display filters");
    let matches: Vec<_> = findings
        .iter()
        .filter(|finding| {
            finding["pattern_id"] == "data_dead_end"
                && finding["subject"]["qualified_name"] == "helpers.py::_unused_helper"
        })
        .collect();
    assert_eq!(matches.len(), 1, "one proven private helper: {findings:?}");
    matches[0]
}

fn path_entry<'a>(report: &'a Value, key: &str) -> &'a Value {
    authority(report)["inventory"]["entries"]
        .as_array()
        .expect("inventory entries")
        .iter()
        .find(|entry| entry["path_key"] == key)
        .unwrap_or_else(|| panic!("missing exact path key {key}"))
}

#[test]
fn clean_selected_source_has_closed_inventory_and_full_zero_counts() {
    let dir = project(&[("main.py", b"pass\n")]);
    let output = run(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    let a = authority(&report);
    assert_eq!(a["execution"], "complete");
    assert_eq!(a["outcome"], "clean");
    assert_eq!(a["findings_present"], false);
    assert_eq!(a["findings"]["total"], 0);
    assert_eq!(a["findings"]["all"].as_array().unwrap().len(), 0);
    for key in ["discovered", "selected", "read", "parsed"] {
        assert_eq!(a["inventory"][key], 1, "{key}");
    }
    let entry = path_entry(&report, "6d61696e2e7079");
    assert_eq!(entry["disposition"], "selected");
    assert_eq!(entry["read"], true);
    assert_eq!(entry["parsed"], true);
    assert_eq!(a["provenance"]["config"]["origin"], "default");
    assert_eq!(a["scope"]["gate_threshold"]["min_severity"], "critical");
    assert_eq!(report["outcome"], a["outcome"]);
}

#[test]
fn consumed_source_digest_and_private_cause_are_analyzer_records() {
    let dir = project(&[("helpers.py", HELPER), ("main.py", MAIN)]);
    let output = run(dir.path(), &[]);
    assert_eq!(
        output.status.code(),
        Some(0),
        "warning is below default critical gate"
    );
    let report = json(&output);
    let a = authority(&report);
    assert_eq!(a["outcome"], "findings");
    assert_eq!(a["gate"]["verdict"], "pass");
    assert_eq!(a["inventory"]["selected"], 2);
    assert_eq!(a["inventory"]["read"], 2);
    assert_eq!(a["inventory"]["parsed"], 2);
    let all = a["findings"]["all"].as_array().expect("full findings");
    assert_eq!(a["findings"]["total"], all.len());
    for (pattern, count) in a["findings"]["by_pattern"].as_object().unwrap() {
        assert_eq!(
            count.as_u64().unwrap() as usize,
            all.iter()
                .filter(|finding| finding["pattern_id"] == pattern.as_str())
                .count(),
            "pattern count for {pattern}"
        );
    }
    for (severity, count) in a["findings"]["by_severity"].as_object().unwrap() {
        assert_eq!(
            count.as_u64().unwrap() as usize,
            all.iter()
                .filter(|finding| finding["severity"] == severity.as_str())
                .count(),
            "severity count for {severity}"
        );
    }
    let input = a["provenance"]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["path_key"] == "68656c706572732e7079")
        .expect("consumed helper input");
    assert_eq!(
        input["raw_sha256"],
        "sha256:39db0d736f749957a00acd5fe3717106cfc3d56caa3d3a7b09d8d028585724cd"
    );
    assert_eq!(input["bytes_consumed"], HELPER.len());
    let finding = private_finding(&report);
    assert_eq!(finding["identity_status"], "stable");
    assert!(finding["semantic_id"]
        .as_str()
        .unwrap()
        .starts_with("sha256:"));
    assert_eq!(finding["subject"]["path_key"], "68656c706572732e7079");
    assert_eq!(finding["subject"]["language"], "python");
    assert_eq!(finding["subject"]["construct_kind"], "function");
    assert_eq!(finding["cause"]["key_version"], 1);
    assert_eq!(
        finding["cause"]["parts"]["unconsumed_relation"],
        "unconsumed_private_symbol"
    );
}

#[test]
fn private_semantic_identity_survives_re_root_line_shift_and_earlier_finding() {
    let base = project(&[("helpers.py", HELPER), ("main.py", MAIN)]);
    let shifted = project(&[
        ("aaa.py", b"def _earlier():\n    return 2\n"),
        ("helpers.py", b"\n\ndef _unused_helper():\n    return 1\n"),
        ("main.py", MAIN),
    ]);
    let first = json(&run(base.path(), &[]));
    let second = json(&run(shifted.path(), &[]));
    let a = private_finding(&first);
    let b = private_finding(&second);
    assert_eq!(a["semantic_id"], b["semantic_id"]);
    assert_ne!(a["observation_id"], b["observation_id"]);
    assert_ne!(a["range"]["start_line"], b["range"]["start_line"]);
}

#[test]
fn public_helper_is_not_a_proven_private_dead_end() {
    let dir = project(&[
        ("helpers.py", b"def unused_helper():\n    return 1\n"),
        ("main.py", MAIN),
    ]);
    let report = json(&run(dir.path(), &[]));
    assert!(authority(&report)["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .all(|finding| {
            finding["pattern_id"] != "data_dead_end"
                || finding["subject"]["qualified_name"] != "helpers.py::unused_helper"
        }));
}

#[test]
fn called_private_helper_is_not_reported_as_unused() {
    let dir = project(&[
        ("helpers.py", HELPER),
        ("main.py", b"from helpers import _unused_helper\n\ndef main():\n    return _unused_helper()\n\nif __name__ == \"__main__\":\n    main()\n"),
    ]);
    let report = json(&run(dir.path(), &[]));
    assert!(authority(&report)["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .all(|finding| {
            finding["pattern_id"] != "data_dead_end"
                || finding["subject"]["qualified_name"] != "helpers.py::_unused_helper"
        }));
}

#[test]
fn explicit_gate_changes_exit_and_display_filter_cannot_change_full_authority() {
    let dir = project(&[("helpers.py", HELPER), ("main.py", MAIN)]);
    let default = run(dir.path(), &[]);
    let gated = run(
        dir.path(),
        &["--gate-severity", "warning", "--gate-confidence", "low"],
    );
    let filtered = run(dir.path(), &["--checks", "isolated_cluster"]);
    assert_eq!(default.status.code(), Some(0));
    assert_eq!(gated.status.code(), Some(2));
    assert_eq!(filtered.status.code(), Some(0));
    let a = json(&default);
    let g = json(&gated);
    let f = json(&filtered);
    assert_eq!(authority(&a)["outcome"], "findings");
    assert_eq!(authority(&g)["gate"]["verdict"], "fail");
    assert_eq!(
        authority(&g)["scope"]["gate_threshold"]["min_severity"],
        "warning"
    );
    assert_eq!(
        authority(&g)["provenance"]["config"]["effective"]["gate_threshold"],
        authority(&g)["scope"]["gate_threshold"]
    );
    assert_eq!(authority(&a)["findings"], authority(&f)["findings"]);
    assert_eq!(
        authority(&a)["report_identity"],
        authority(&f)["report_identity"]
    );
    assert_eq!(authority(&a)["gate"], authority(&f)["gate"]);
    assert_eq!(f["full_findings_total"], authority(&f)["findings"]["total"]);
    assert_eq!(f["displayed_findings_total"], 0);
}

#[test]
fn incomplete_parse_keeps_private_positive_and_ranged_loss() {
    let dir = project(&[
        ("helpers.py", HELPER),
        ("main.py", MAIN),
        ("broken.py", b"def broken(:\n    pass\n"),
    ]);
    let output = run(
        dir.path(),
        &["--gate-severity", "warning", "--gate-confidence", "low"],
    );
    assert_eq!(
        output.status.code(),
        Some(3),
        "incompleteness dominates gate"
    );
    let report = json(&output);
    let a = authority(&report);
    assert_eq!(a["execution"], "incomplete");
    assert_eq!(a["outcome"], "incomplete");
    assert_eq!(a["inventory"]["selected"], 3);
    assert_eq!(a["inventory"]["rejected"], 1);
    assert!(a["losses"].as_array().unwrap().iter().any(|loss| {
        loss["reason"] == "parse_recovery"
            && loss["scope_paths"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "62726f6b656e2e7079")
            && !loss["ranges"].as_array().unwrap().is_empty()
    }));
    private_finding(&report);
}

#[test]
fn invalid_implicit_config_is_hard_error_without_output_file() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"languages: [python"),
    ]);
    let out = dir.path().join("report.json");
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json", "-o"])
        .arg(&out)
        .output()
        .expect("analyze invalid config");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(!out.exists());
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("config.yaml") && stderr.contains("parse"),
        "{stderr}"
    );
}

#[test]
fn invalid_explicit_config_takes_precedence_over_valid_implicit() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"languages: [python]\n"),
        ("bad.yaml", b"languages: [python"),
    ]);
    let config = dir.path().join("bad.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json", "--config"])
        .arg(config)
        .output()
        .expect("analyze explicit config");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("bad.yaml") && stderr.contains("parse"),
        "{stderr}"
    );
}

#[test]
fn invalid_known_type_and_unknown_field_fail_closed() {
    for config in [
        b"languages: 42\n".as_slice(),
        b"languages: [python]\nfuture_field: value\n".as_slice(),
    ] {
        let dir = project(&[("main.py", b"pass\n"), (".flowspec/config.yaml", config)]);
        let output = run(dir.path(), &[]);
        assert_eq!(
            output.status.code(),
            Some(1),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty());
        assert!(String::from_utf8_lossy(&output.stderr).contains("config.yaml"));
    }
}

#[test]
fn valid_explicit_config_overrides_malformed_implicit_and_records_origin() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"languages: [python"),
        ("good.yaml", b"languages: [python]\n"),
    ]);
    let config = dir.path().join("good.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json", "--config"])
        .arg(config)
        .output()
        .expect("analyze explicit valid config");
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    assert_eq!(
        authority(&report)["provenance"]["config"]["origin"],
        "explicit"
    );
    assert_eq!(authority(&report)["inventory"]["selected"], 1);
}

#[test]
fn output_file_contains_same_authority_as_stdout() {
    let dir = project(&[("helpers.py", HELPER), ("main.py", MAIN)]);
    let destination = TempDir::new().expect("independent output directory");
    let path = destination.path().join("out.json");
    let stdout = json(&run(dir.path(), &[]));
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(dir.path())
        .args(["--format", "json", "-o"])
        .arg(&path)
        .output()
        .expect("write report");
    assert_eq!(output.status.code(), Some(0));
    assert!(output.stdout.is_empty());
    let written: Value = serde_json::from_slice(&fs::read(path).expect("written report")).unwrap();
    assert_eq!(
        authority(&stdout)["report_identity"],
        authority(&written)["report_identity"]
    );
}

#[test]
fn distinct_non_utf8_linux_paths_keep_distinct_inventory_keys() {
    let dir = project(&[]);
    for byte in [0xfe, 0xff] {
        let path = PathBuf::from(OsString::from_vec(vec![byte, b'.', b'p', b'y']));
        fs::write(dir.path().join(path), b"pass\n").expect("write non-UTF8 source");
    }
    let report = json(&run(dir.path(), &[]));
    assert_eq!(path_entry(&report, "fe2e7079")["disposition"], "selected");
    assert_eq!(path_entry(&report, "ff2e7079")["disposition"], "selected");
    assert_eq!(authority(&report)["inventory"]["selected"], 2);
}

#[test]
fn empty_scope_is_clean_only_after_closed_discovery() {
    let dir = project(&[]);
    let output = run(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(0));
    let report = json(&output);
    let a = authority(&report);
    assert_eq!(a["execution"], "complete");
    assert_eq!(a["outcome"], "clean");
    assert_eq!(a["inventory"]["discovered"], 0);
    assert_eq!(a["inventory"]["selected"], 0);
    assert!(a["losses"].as_array().unwrap().is_empty());
}

#[test]
fn valid_implicit_comment_config_is_consumed_control() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"# no settings\n"),
    ]);
    let report = json(&run(dir.path(), &[]));
    let a = authority(&report);
    assert_eq!(a["provenance"]["config"]["origin"], "implicit");
    assert_eq!(a["scope"]["policy_origin"], "implicit");
    assert_eq!(a["inventory"]["selected"], 1);
    assert!(a["provenance"]["controls"]
        .as_array()
        .unwrap()
        .iter()
        .any(|control| {
            control["path_key"] == "2e666c6f77737065632f636f6e6669672e79616d6c"
                && control["kind"] == "effective_config"
                && control["raw_sha256"]
                    .as_str()
                    .is_some_and(|digest| digest.starts_with("sha256:"))
        }));
}

#[test]
fn empty_language_policy_autodetects_selected_python() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"languages: []\n"),
    ]);
    let report = json(&run(dir.path(), &[]));
    let a = authority(&report);
    assert_eq!(a["inventory"]["selected"], 1);
    assert_eq!(a["inventory"]["parsed"], 1);
    assert!(a["scope"]["effective_languages"]
        .as_array()
        .unwrap()
        .contains(&Value::String("python".into())));
}

#[test]
fn policy_exclusion_and_unsupported_file_are_distinct_visible_dispositions() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        ("vendor/drop.py", b"import sys\n"),
        ("note.xyz", b"opaque\n"),
        (".flowspec/config.yaml", b"exclude:\n  - vendor/\n"),
    ]);
    let report = json(&run(dir.path(), &[]));
    let a = authority(&report);
    assert_eq!(
        path_entry(&report, "76656e646f722f64726f702e7079")["disposition"],
        "policy_excluded"
    );
    assert_eq!(
        path_entry(&report, "6e6f74652e78797a")["disposition"],
        "unsupported"
    );
    assert_eq!(a["inventory"]["discovered"], 3);
    assert_eq!(a["inventory"]["selected"], 1);
    assert_eq!(a["inventory"]["policy_excluded"], 1);
    assert_eq!(a["inventory"]["unsupported"], 1);
}

#[test]
fn missing_explicit_config_and_directory_config_are_hard_errors() {
    let missing = project(&[("main.py", b"pass\n")]);
    let missing_path = missing.path().join("missing.yaml");
    let output = Command::new(env!("CARGO_BIN_EXE_flowspec"))
        .arg("analyze")
        .arg(missing.path())
        .args(["--format", "json", "--config"])
        .arg(&missing_path)
        .output()
        .expect("missing explicit config");
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    assert!(String::from_utf8_lossy(&output.stderr).contains("missing.yaml"));

    let directory = project(&[("main.py", b"pass\n")]);
    let config_dir = directory.path().join(".flowspec/config.yaml");
    fs::create_dir_all(&config_dir).expect("directory at config path");
    let output = run(directory.path(), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr).to_lowercase();
    assert!(
        stderr.contains("config.yaml") && stderr.contains("read"),
        "{stderr}"
    );
}

#[test]
fn parent_traversal_exclusion_is_rejected_as_config_error() {
    let dir = project(&[
        ("main.py", b"pass\n"),
        (".flowspec/config.yaml", b"exclude:\n  - ../escape\n"),
    ]);
    let output = run(dir.path(), &[]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("config.yaml") && stderr.contains("../escape"),
        "{stderr}"
    );
}
