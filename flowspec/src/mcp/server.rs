//! Minimal MCP server — JSON-RPC 2.0 over stdio.
//!
//! Exposes `flowspec_analyze` as an MCP tool. Read line-by-line from stdin,
//! write JSON responses to stdout, log to stderr.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::Path;

/// Run the MCP server. Blocks until stdin closes.
pub fn run() -> io::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let method = req.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = req.get("id").cloned().unwrap_or(Value::Null);

        let response: Value = match method {
            "initialize" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": {
                    "protocolVersion": "2024-11-05",
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "flowspec", "version": "0.1.0" }
                }
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0", "id": id,
                "result": { "tools": [{
                    "name": "flowspec_analyze",
                    "description": "Analyze a codebase — returns entities, flows, diagnostics summary",
                    "inputSchema": {
                        "type": "object",
                        "properties": {
                            "path": { "type": "string", "description": "Project path to analyze" }
                        },
                        "required": ["path"]
                    }
                }]}
            }),
            "tools/call" => {
                let params = req.get("params").cloned().unwrap_or(json!({}));
                let name = params.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let args = params.get("arguments").cloned().unwrap_or(json!({}));
                let text = match name {
                    "flowspec_analyze" => {
                        let p = args.get("path").and_then(|p| p.as_str()).unwrap_or(".");
                        handle_analyze(Path::new(p))
                    }
                    _ => format!("Unknown tool: {}", name),
                };
                json!({ "jsonrpc": "2.0", "id": id,
                    "result": { "content": [{ "type": "text", "text": text }] } })
            }
            _ => {
                if id == Value::Null {
                    continue; // notification — no response
                }
                json!({ "jsonrpc": "2.0", "id": id,
                    "error": { "code": -32601, "message": "Method not found" } })
            }
        };
        writeln!(out, "{}", response)?;
        out.flush()?;
    }
    Ok(())
}

/// Run flowspec analyze + return a text summary.
fn handle_analyze(path: &Path) -> String {
    let config = crate::Config::load(path, None).unwrap_or_default();
    match crate::analyze(path, &config, &[]) {
        Ok(result) => {
            let m = &result.manifest;
            let top = m
                .diagnostics
                .iter()
                .take(5)
                .map(|d| {
                    format!(
                        "  {} — {} ({})",
                        d.pattern,
                        d.message.chars().take(80).collect::<String>(),
                        d.loc
                    )
                })
                .collect::<Vec<_>>()
                .join("\n");
            format!(
                "Flowspec Analysis | Flows: {} | Diagnostics: {}\n\n{}",
                m.flows.len(),
                m.diagnostics.len(),
                top
            )
        }
        Err(e) => format!("Analysis failed: {}", e),
    }
}
