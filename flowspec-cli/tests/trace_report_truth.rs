use assert_cmd::Command;
use serde_json::Value;
#[test]
fn trace_report_has_exact_hops_and_depth_continuation() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        "def main():\n    first()\ndef first():\n    last()\ndef last():\n    return 1\n",
    )
    .unwrap();
    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "--format",
            "json",
            "trace",
            dir.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--depth",
            "1",
            "--report",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "depth loss is exit 3, not clean or unsupported-argument exit"
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["execution"], "incomplete");
    assert_eq!(report["paths"][0]["hops"][0]["edge_kind"], "calls");
    assert_eq!(report["paths"][0]["hops"].as_array().unwrap().len(), 1);
    assert!(!report["continuation"].as_array().unwrap().is_empty());
}
#[test]
fn packaged_mcp_trace_returns_the_same_exact_query_state() {
    use std::io::Write;
    use std::process::{Command as Process, Stdio};
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("app.py"),
        "def main():\n    first()\ndef first():\n    last()\ndef last():\n    return 1\n",
    )
    .unwrap();
    let binary = assert_cmd::cargo::cargo_bin("flowspec");
    let mut child = Process::new(binary)
        .args(["mcp", "serve", "--root", dir.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        let mut stdin = child.stdin.take().unwrap();
        writeln!(stdin,"{}",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"flowspec_trace","arguments":{"path":".","symbol":"main","depth":1}}})).unwrap();
    }
    let output = child.wait_with_output().unwrap();
    let response: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        response["result"]["structuredContent"]["execution"],
        "incomplete"
    );
    assert_eq!(
        response["result"]["structuredContent"]["paths"][0]["hops"][0]["edge_kind"],
        "calls"
    );
    let text = response["result"]["content"][0]["text"].as_str().unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(text).unwrap(),
        response["result"]["structuredContent"]
    );
}
#[test]
fn parser_recovery_at_requested_symbol_cannot_be_a_complete_zero_flow_trace() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.py"), "def main():\n    value =\n").unwrap();
    let output = Command::cargo_bin("flowspec")
        .unwrap()
        .args([
            "--format",
            "json",
            "trace",
            dir.path().to_str().unwrap(),
            "--symbol",
            "main",
            "--report",
        ])
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "selected parser recovery is retained by zero-path queries"
    );
    let report: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["execution"], "incomplete");
    assert!(!report["losses"].as_array().unwrap().is_empty());
}
