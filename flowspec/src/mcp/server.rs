//! Local MCP 2025-06-18, newline-delimited JSON-RPC over stdio.
//! stdout contains protocol frames only; every analysis read has explicit root authority.
use crate::{Config, FlowspecError, ReportOptions};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

const PROTOCOL: &str = "2025-06-18";

fn refusal(reason: impl Into<String>) -> FlowspecError {
    FlowspecError::Config {
        reason: reason.into(),
        suggestion: "use a real directory and regular files within --root; symlinks are refused"
            .into(),
    }
}

/// Canonical read boundary. All symlinks are refused, including in-root links.
/// This local server assumes its workspace is not concurrently mutated by an adversary.
struct Workspace {
    root: PathBuf,
    config: Option<PathBuf>,
}
impl Workspace {
    fn new(root: &Path, config: Option<&Path>) -> Result<Self, FlowspecError> {
        let root = root
            .canonicalize()
            .map_err(|e| refusal(format!("workspace root: {e}")))?;
        if !root.is_dir() {
            return Err(refusal("workspace root must be a directory"));
        }
        let mut workspace = Self { root, config: None };
        if let Some(config) = config {
            workspace.config = Some(workspace.path(config)?);
        }
        Ok(workspace)
    }
    fn path(&self, requested: &Path) -> Result<PathBuf, FlowspecError> {
        if requested.as_os_str().is_empty() {
            return Err(refusal("empty path"));
        }
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            self.root.join(requested)
        };
        if !joined.starts_with(&self.root) {
            return Err(refusal("path outside workspace root"));
        }
        // Check lexical components before canonicalization; '..' cannot escape authority.
        let mut checked = self.root.clone();
        for component in joined
            .strip_prefix(&self.root)
            .map_err(|_| refusal("outside root"))?
            .components()
        {
            match component {
                std::path::Component::Normal(name) => checked.push(name),
                std::path::Component::CurDir => continue,
                _ => return Err(refusal("parent traversal is refused")),
            }
            let meta = std::fs::symlink_metadata(&checked)
                .map_err(|e| refusal(format!("{}: {e}", checked.display())))?;
            if meta.file_type().is_symlink() {
                return Err(refusal(format!("symlink refused: {}", checked.display())));
            }
        }
        let canonical = joined
            .canonicalize()
            .map_err(|e| refusal(format!("{}: {e}", joined.display())))?;
        if !canonical.starts_with(&self.root) {
            return Err(refusal("canonical path outside workspace root"));
        }
        Ok(canonical)
    }
    fn descendants(directory: &Path) -> Result<(), FlowspecError> {
        for entry in std::fs::read_dir(directory)
            .map_err(|e| refusal(format!("{}: {e}", directory.display())))?
        {
            let entry = entry.map_err(|e| refusal(format!("directory entry: {e}")))?;
            let meta = entry
                .file_type()
                .map_err(|e| refusal(format!("file type: {e}")))?;
            if meta.is_symlink() {
                return Err(refusal(format!(
                    "symlink refused: {}",
                    entry.path().display()
                )));
            }
            if meta.is_dir() {
                Self::descendants(&entry.path())?;
            }
        }
        Ok(())
    }
    fn analyze_result(&self, args: &Value) -> Result<crate::AnalysisResult, FlowspecError> {
        let path = self.path(Path::new(args["path"].as_str().expect("validated path")))?;
        if !path.is_dir() {
            return Err(refusal("analysis target must be a directory"));
        }
        Self::descendants(&path)?;
        // Validate implicit policy even though Config::load would otherwise follow a link.
        let default_config = path.join(".flowspec/config.yaml");
        if std::fs::symlink_metadata(&default_config).is_ok() {
            self.path(&default_config)?;
        }
        if let Some(config) = &self.config {
            self.path(config)?;
        }
        let mut config = Config::load(&path, self.config.as_deref())?;
        config.read_root = Some(self.root.clone());
        let languages = crate::commands::normalize_languages(&strings(args.get("language")));
        let options = ReportOptions {
            checks: strings(args.get("checks")),
            severity: args
                .get("severity")
                .and_then(Value::as_str)
                .and_then(crate::Severity::from_str_checked),
            confidence: args
                .get("confidence")
                .and_then(Value::as_str)
                .and_then(crate::Confidence::from_str_checked),
            gate_severity: args
                .get("gate_severity")
                .and_then(Value::as_str)
                .and_then(crate::Severity::from_str_checked)
                .unwrap_or(crate::Severity::Critical),
            gate_confidence: args
                .get("gate_confidence")
                .and_then(Value::as_str)
                .and_then(crate::Confidence::from_str_checked)
                .unwrap_or(crate::Confidence::Low),
        };
        let result = crate::diagnose_report_with_analysis_options(
            &path,
            &config,
            &languages,
            &options,
            &crate::AnalysisOptions {
                mode: if args["incremental"] == true {
                    crate::cache::AnalysisMode::Incremental
                } else {
                    crate::cache::AnalysisMode::Full
                },
                ..Default::default()
            },
        )?;
        let serialized =
            serde_json::to_string(&result.manifest).map_err(|error| FlowspecError::Manifest {
                reason: error.to_string(),
            })?;
        crate::manifest::validate_manifest_size(&serialized, result.source_bytes, "json")?;
        Ok(result)
    }
    fn analyze(&self, args: &Value) -> Result<crate::AnalysisReport, FlowspecError> {
        Ok(self.analyze_result(args)?.manifest)
    }
    fn trace(&self, args: &Value) -> Result<crate::commands::TraceOutput, FlowspecError> {
        let root = self.path(Path::new(args["path"].as_str().expect("validated path")))?;
        let result = self.analyze_result(args)?;
        let direction = match args["direction"].as_str().unwrap_or("forward") {
            "backward" => crate::commands::TraceDirection::Backward,
            "both" => crate::commands::TraceDirection::Both,
            _ => crate::commands::TraceDirection::Forward,
        };
        crate::commands::exact_trace_output(
            &root,
            &result,
            args["symbol"].as_str().expect("validated symbol"),
            args["depth"].as_u64().unwrap_or(10) as usize,
            direction,
        )
    }
}
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect()
}
fn valid_args(args: &Value) -> bool {
    let Some(map) = args.as_object() else {
        return false;
    };
    if !args
        .get("path")
        .is_some_and(|p| p.is_string() && !p.as_str().unwrap().is_empty())
    {
        return false;
    }
    if strings(args.get("language"))
        .iter()
        .any(|language| crate::commands::validate_language(language).is_err())
        || crate::commands::validate_check_patterns(&strings(args.get("checks"))).is_err()
    {
        return false;
    }
    map.iter().all(|(key, value)| match key.as_str() {
        "path" => value.is_string(),
        "incremental" => value.is_boolean(),
        "language" | "checks" => value
            .as_array()
            .is_some_and(|a| a.iter().all(Value::is_string)),
        "severity" | "gate_severity" => value
            .as_str()
            .is_some_and(|v| crate::Severity::from_str_checked(v).is_some()),
        "confidence" | "gate_confidence" => value
            .as_str()
            .is_some_and(|v| crate::Confidence::from_str_checked(v).is_some()),
        _ => false,
    })
}
fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
fn tools() -> Value {
    let schema = json!({"type":"object","properties":{
        "path":{"type":"string","minLength":1},
        "incremental":{"type":"boolean"},
        "language":{"type":"array","items":{"type":"string"}},
        "checks":{"type":"array","items":{"type":"string"}},
        "severity":{"type":"string","enum":["critical","warning","info"]},
        "confidence":{"type":"string","enum":["high","moderate","low"]},
        "gate_severity":{"type":"string","enum":["critical","warning","info"]},
        "gate_confidence":{"type":"string","enum":["high","moderate","low"]}
    },"required":["path"],"additionalProperties":false});
    let mut trace_schema = schema.clone();
    trace_schema["properties"]["symbol"] = json!({"type":"string","minLength":1});
    trace_schema["properties"]["depth"] = json!({"type":"integer","minimum":0,"maximum":256});
    trace_schema["properties"]["direction"] =
        json!({"type":"string","enum":["forward","backward","both"]});
    trace_schema["required"] = json!(["path", "symbol"]);
    json!({"tools":[{"name":"flowspec_analyze","description":"Full AnalysisReport; Critical/Low gate, display filters preserve authority findings", "inputSchema":schema,
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}},
        {"name":"flowspec_trace","description":"Exact ordered graph hops with scoped uncertainty and depth continuation","inputSchema":trace_schema,
        "annotations":{"readOnlyHint":true,"destructiveHint":false,"openWorldHint":false}}]})
}
fn dispatch(workspace: &Workspace, request: Value) -> Option<Value> {
    if !request.is_object()
        || request["jsonrpc"] != "2.0"
        || !request["method"].is_string()
        || request
            .get("id")
            .is_some_and(|id| !(id.is_string() || id.is_number() || id.is_null()))
    {
        return Some(error(Value::Null, -32600, "Invalid Request"));
    }
    // Notifications never execute tools and never generate responses.
    let id = request.get("id")?.clone();
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return Some(error(id, -32602, "Invalid params"));
    }
    let result = match request["method"].as_str().unwrap() {
        "initialize" => {
            if !params["protocolVersion"].is_string()
                || !params["capabilities"].is_object()
                || !params["clientInfo"].is_object()
                || !params["clientInfo"]["name"].is_string()
                || !params["clientInfo"]["version"].is_string()
            {
                return Some(error(id, -32602, "Invalid initialize params"));
            }
            json!({"protocolVersion":PROTOCOL,"capabilities":{"tools":{}},"serverInfo":{"name":"flowspec","version":env!("CARGO_PKG_VERSION")}})
        }
        "ping" => json!({}),
        "tools/list" => {
            if params.get("cursor").is_some() {
                return Some(error(id, -32602, "No pagination cursor is supported"));
            }
            tools()
        }
        "tools/call" => {
            let arguments = &params["arguments"];
            let traced = params["name"] == "flowspec_trace";
            let mut common = arguments.clone();
            if traced {
                if arguments["symbol"].as_str().is_none_or(|s| s.is_empty())
                    || arguments
                        .get("depth")
                        .is_some_and(|d| d.as_u64().is_none_or(|n| n > 256))
                    || arguments.get("direction").is_some_and(|d| {
                        !d.as_str()
                            .is_some_and(|s| matches!(s, "forward" | "backward" | "both"))
                    })
                {
                    return Some(error(id, -32602, "Invalid trace arguments"));
                }
                if let Some(map) = common.as_object_mut() {
                    map.remove("symbol");
                    map.remove("depth");
                    map.remove("direction");
                }
            }
            if (!traced && params["name"] != "flowspec_analyze") || !valid_args(&common) {
                return Some(error(id, -32602, "Unknown tool or invalid arguments"));
            }
            let report = if traced {
                workspace
                    .trace(arguments)
                    .map(|report| serde_json::to_value(report).expect("serializable trace"))
            } else {
                workspace
                    .analyze(arguments)
                    .map(|report| serde_json::to_value(report).expect("serializable report"))
            };
            match report {
                Ok(report) => {
                    let structured = serde_json::to_value(report).expect("serializable report");
                    let text = serde_json::to_string(&structured).expect("serializable report");
                    json!({"structuredContent":structured,"content":[{"type":"text","text":text}],"isError":false})
                }
                Err(e) => json!({"content":[{"type":"text","text":e.to_string()}],"isError":true}),
            }
        }
        _ => return Some(error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":result}))
}

/// Serve until EOF. Explicit workspace authority is required before reading stdin.
pub fn run_scoped(root: &Path, config: Option<&Path>) -> Result<(), FlowspecError> {
    let workspace = Workspace::new(root, config)?;
    let stdin = io::stdin();
    let mut stdout = io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line.map_err(|source| FlowspecError::Io {
            path: "<stdin>".into(),
            source,
        })?;
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str(&line) {
            Ok(request) => dispatch(&workspace, request),
            Err(_) => Some(error(Value::Null, -32700, "Parse error")),
        };
        if let Some(response) = response {
            writeln!(stdout, "{response}")
                .and_then(|_| stdout.flush())
                .map_err(|source| FlowspecError::Io {
                    path: "<stdout>".into(),
                    source,
                })?;
        }
    }
    Ok(())
}

/// Compatibility entrypoint refuses implicit ambient read authority.
#[deprecated(note = "explicit root authority required; use run_scoped")]
pub fn run() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        "use flowspec mcp serve --root PATH",
    ))
}
