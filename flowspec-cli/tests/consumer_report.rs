use assert_cmd::Command;
use serde_json::{json, Value};
use std::fs;
use std::io::Write;
use std::process::{Command as Process, Stdio};
use tempfile::TempDir;

fn project(source: &str) -> TempDir {
    let p = TempDir::new().unwrap();
    fs::write(p.path().join("example.py"), source).unwrap();
    p
}
fn diagnose(p: &TempDir, args: &[&str]) -> std::process::Output {
    Command::cargo_bin("flowspec")
        .unwrap()
        .arg("diagnose")
        .arg(p.path())
        .args(["--format", "json"])
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn diagnose_keeps_full_report_and_selected_parse_loss() {
    let p = project("def broken(:\n    return (\n");
    let out = diagnose(&p, &[]);
    assert_eq!(
        out.status.code(),
        Some(3),
        "selected parse loss cannot become clean"
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(report["execution"], "incomplete");
    assert!(report["authority"]["losses"].as_array().unwrap().len() > 0);
}
#[test]
fn diagnose_json_is_report_even_for_complete_clean() {
    let p = project("# clean empty module\n");
    let out = diagnose(&p, &[]);
    assert_eq!(out.status.code(), Some(0));
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(report.is_object(), "full report envelope required");
    assert_eq!(report["execution"], "complete");
    assert!(report["authority"]["producer"]["build_sha"].is_string());
}
#[test]
fn diagnose_gate_flags_are_separate_from_display() {
    let p = project("# clean\n");
    let out = diagnose(
        &p,
        &[
            "--gate-severity",
            "info",
            "--gate-confidence",
            "low",
            "--severity",
            "critical",
        ],
    );
    assert_eq!(
        out.status.code(),
        Some(0),
        "valid gate controls must be accepted"
    );
    let report: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert!(report["authority"].is_object());
}
fn frames(root: &std::path::Path, requests: Vec<Value>) -> (std::process::ExitStatus, Vec<Value>) {
    let mut child = Process::new(assert_cmd::cargo::cargo_bin("flowspec"))
        .args(["mcp", "serve", "--root"])
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    for request in requests {
        writeln!(input, "{request}").unwrap();
    }
    drop(input);
    let output = child.wait_with_output().unwrap();
    let rows = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    (output.status, rows)
}
#[test]
fn packaged_mcp_full_structured_report_and_notifications() {
    let p = project("# clean\n");
    let (status, rows) = frames(
        p.path(),
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}),
            json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
            json!({"jsonrpc":"2.0","method":"tools/list"}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":"."}}}),
        ],
    );
    assert!(status.success(), "normal packaged MCP entrypoint required");
    assert_eq!(rows.len(), 2, "notifications must never emit responses");
    assert_eq!(rows[0]["result"]["protocolVersion"], "2025-06-18");
    let result = &rows[1]["result"];
    assert_eq!(result["isError"], false);
    assert!(result["structuredContent"]["authority"].is_object());
    assert_eq!(
        serde_json::from_str::<Value>(result["content"][0]["text"].as_str().unwrap()).unwrap(),
        result["structuredContent"]
    );
}
#[test]
fn packaged_mcp_invalid_arguments_and_config_failure_refuse() {
    let p = project("# clean\n");
    fs::create_dir(p.path().join(".flowspec")).unwrap();
    fs::write(p.path().join(".flowspec/config.yaml"), "languages: [").unwrap();
    let (status, rows) = frames(
        p.path(),
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":7}}}),
            json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":"."}}}),
            json!({"jsonrpc":"2.0","id":3,"method":"unknown"}),
        ],
    );
    assert!(status.success());
    assert_eq!(rows[0]["error"]["code"], -32602);
    assert_eq!(rows[1]["result"]["isError"], true);
    assert!(rows[1]["result"].get("structuredContent").is_none());
    assert_eq!(rows[2]["error"]["code"], -32601);
}

#[test]
fn private_positive_gate_survives_hidden_display_and_equal_options_identity() {
    let p = project("def _unused():\n    return 1\n");
    let hidden = diagnose(&p, &["--severity", "critical"]);
    assert_eq!(hidden.status.code(), Some(2));
    let report: Value = serde_json::from_slice(&hidden.stdout).unwrap();
    assert_eq!(report["execution"], "complete");
    assert_eq!(report["authority"]["findings"]["total"], 1);
    assert_eq!(report["authority"]["gate"]["verdict"], "fail");
    assert_eq!(report["authority"]["display"]["indices"], json!([]));
    let analyzed = Command::cargo_bin("flowspec")
        .unwrap()
        .arg("analyze")
        .arg(p.path())
        .args([
            "--format",
            "json",
            "--gate-severity",
            "info",
            "--severity",
            "critical",
        ])
        .output()
        .unwrap();
    let analyze: Value = serde_json::from_slice(&analyzed.stdout).unwrap();
    assert_eq!(report["authority"], analyze["authority"]);
    let (_, rows) = frames(
        p.path(),
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":".","gate_severity":"info","severity":"critical"}}}),
        ],
    );
    assert_eq!(rows[0]["result"]["isError"], false);
    assert_eq!(
        rows[0]["result"]["structuredContent"]["authority"],
        report["authority"]
    );
}

#[test]
fn mcp_full_catalog_retains_more_than_five_genuine_findings() {
    let source = (0..8)
        .map(|i| format!("def _unused_{i}():\n    return {i}\n"))
        .collect::<String>();
    let p = project(&source);
    let (_, rows) = frames(
        p.path(),
        vec![
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":"."}}}),
        ],
    );
    let result = &rows[0]["result"];
    assert_eq!(
        result["isError"], false,
        "catalog should be delivered, not truncated or defaulted"
    );
    let report = &result["structuredContent"];
    assert_eq!(report["authority"]["findings"]["total"], 8);
    assert_eq!(
        report["authority"]["findings"]["all"]
            .as_array()
            .unwrap()
            .len(),
        8
    );
    assert!(report["authority"]["findings"]["all"]
        .as_array()
        .unwrap()
        .iter()
        .all(|f| f["identity_status"] == "stable"));
}

#[test]
fn mcp_refuses_outside_root_and_symlink_source_and_policy() {
    use std::os::unix::fs::symlink;
    let inside = project("# clean\n");
    let outside = project("def _private_outside():\n    return 1\n");
    let request = |path: String| json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"flowspec_analyze","arguments":{"path":path}}});
    let (_, rows) = frames(
        inside.path(),
        vec![request(outside.path().display().to_string())],
    );
    assert_eq!(rows[0]["result"]["isError"], true);
    symlink(
        outside.path().join("example.py"),
        inside.path().join("escape.py"),
    )
    .unwrap();
    let (_, rows) = frames(inside.path(), vec![request(".".into())]);
    assert_eq!(rows[0]["result"]["isError"], true);
    fs::remove_file(inside.path().join("escape.py")).unwrap();
    fs::create_dir(inside.path().join(".flowspec")).unwrap();
    fs::write(outside.path().join("config.yaml"), "languages: [python]\n").unwrap();
    symlink(
        outside.path().join("config.yaml"),
        inside.path().join(".flowspec/config.yaml"),
    )
    .unwrap();
    let (_, rows) = frames(inside.path(), vec![request(".".into())]);
    assert_eq!(rows[0]["result"]["isError"], true);
    assert!(rows[0]["result"].get("structuredContent").is_none());
}

#[test]
fn diagnose_hard_error_never_writes_output() {
    let p = project("# clean\n");
    fs::create_dir(p.path().join(".flowspec")).unwrap();
    fs::write(p.path().join(".flowspec/config.yaml"), "languages: [").unwrap();
    let out = diagnose(&p, &[]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}
