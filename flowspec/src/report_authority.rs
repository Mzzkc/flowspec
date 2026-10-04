//! Analyzer-origin records for the additive JSON report contract.
//! The legacy manifest remains a presentation and compatibility surface.
use crate::config::Config;
use crate::graph::Graph;
use crate::manifest::types::DiagnosticEntry;
use crate::parser::ir::{EdgeKind, SymbolKind};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashMap};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};

pub(crate) const PATTERNS: [&str; 13] = [
    "isolated_cluster",
    "data_dead_end",
    "partial_wiring",
    "orphaned_impl",
    "duplication",
    "contract_mismatch",
    "circular_dependency",
    "layer_violation",
    "incomplete_migration",
    "asymmetric_handling",
    "stale_reference",
    "phantom_dependency",
    "missing_reexport",
];

#[derive(Clone)]
pub(crate) struct InputRecord {
    pub path: PathBuf,
    pub disposition: &'static str,
    pub reason: Option<String>,
    pub language: Option<String>,
    pub content: Option<String>,
    pub read: bool,
    pub parsed: bool,
    pub rejected_reason: Option<String>,
    pub changed_during_run: bool,
}

impl InputRecord {
    pub fn new(path: PathBuf, disposition: &'static str, language: Option<String>) -> Self {
        Self {
            path,
            disposition,
            reason: None,
            language,
            content: None,
            read: false,
            parsed: false,
            rejected_reason: None,
            changed_during_run: false,
        }
    }
}

pub(crate) fn digest(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

pub(crate) fn path_key(root: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.as_os_str()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn hash(domain: &str, value: &Value) -> String {
    // serde_json::Map uses key ordering without preserve_order; all generated
    // object keys are ASCII and values are integral, boolean, string, or null.
    let bytes = serde_json::to_vec(value).expect("canonical report value");
    let mut hasher = Sha256::new();
    hasher.update(format!("flowspec/{domain}/v1\0").as_bytes());
    hasher.update(bytes);
    format!("sha256:{:x}", hasher.finalize())
}

pub(crate) fn loss(reason: &str, path_key: String, range: Option<Value>, stage: &str) -> Value {
    let ranges: Vec<Value> = range.into_iter().collect();
    let id = hash(
        "loss",
        &json!({"reason":reason,"scope_paths":[path_key],"ranges":ranges,"stage":stage}),
    );
    json!({"id":id,"reason":reason,"scope_paths":[path_key],"ranges":ranges,
        "symbols":[],"required_capabilities":[],"affected_patterns":PATTERNS,
        "stage":stage,"continuation":"inspect the affected source and rerun analysis"})
}

fn consumer_closure_blockers(inputs: &[InputRecord], root: &Path, losses: &[Value]) -> Vec<String> {
    let mut blocked = Vec::new();
    for loss in losses {
        let reason = loss["reason"].as_str().unwrap_or("");
        if !matches!(
            reason,
            "discovery_error"
                | "read_error"
                | "changed_during_run"
                | "parse_recovery"
                | "parse_error"
                | "depth_truncation"
                | "unsupported_construct"
                | "unresolved_import"
                | "unresolved_call"
                | "unresolved_reference"
                | "unresolved_type"
        ) {
            continue;
        }
        let paths = loss["scope_paths"].as_array().into_iter().flatten();
        for path in paths {
            let Some(key) = path.as_str() else { continue };
            // Lost selected input may contain a consumer or an escape bridge.
            // No accessibility/interop proof exists here to narrow that scope
            // based on its language or filename. Policy-excluded known paths
            // are outside the declared scope; omitted walker entries are not.
            let relevant_scope = inputs
                .iter()
                .find(|i| path_key(root, &i.path) == key)
                .is_none_or(|input| input.disposition == "selected");
            if relevant_scope {
                blocked.push(key.to_string());
            }
        }
    }
    blocked.sort();
    blocked.dedup();
    blocked
}

fn consumer_closure_loss(finding: &Value, blockers: &[String]) -> Value {
    let mut paths = vec![finding["subject"]["path_key"]
        .as_str()
        .unwrap_or("")
        .to_string()];
    paths.extend(blockers.iter().cloned());
    paths.sort();
    paths.dedup();
    let symbols = vec![finding["subject"]["qualified_name"]
        .as_str()
        .unwrap_or("")
        .to_string()];
    let id = hash(
        "loss",
        &json!({"reason":"semantic_proof_unavailable",
        "scope_paths":paths,"symbols":symbols,"required_capabilities":["consumer_closure"],
        "affected_patterns":["data_dead_end"],"stage":"finding"}),
    );
    json!({"id":id,"reason":"semantic_proof_unavailable","scope_paths":paths,
        "ranges":[],"symbols":symbols,"required_capabilities":["consumer_closure"],
        "affected_patterns":["data_dead_end"],"stage":"finding",
        "continuation":"complete the relevant consumer scope before asserting private absence"})
}

pub(crate) fn point_range(
    path_key: &str,
    line: u64,
    column: u64,
    start_byte: u64,
    end_byte: u64,
) -> Value {
    json!({"path_key":path_key,"start_byte":start_byte,"end_byte":end_byte,
        "start_line":line,"start_column":column,"end_line":line,"end_column":column.saturating_add(end_byte.saturating_sub(start_byte))})
}

fn language(path: &Path) -> Option<&'static str> {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some("py") => Some("python"),
        Some("js" | "jsx" | "mjs" | "cjs") => Some("javascript"),
        Some("ts" | "tsx") => Some("typescript"),
        Some("rs") => Some("rust"),
        _ => None,
    }
}

fn byte_at(content: &str, line: u32, column: u32) -> u64 {
    let offset: usize = content
        .split_inclusive('\n')
        .take(line.saturating_sub(1) as usize)
        .map(str::len)
        .sum();
    (offset + column.saturating_sub(1) as usize).min(content.len()) as u64
}

fn position_at(content: &str, offset: u64) -> (u64, u64) {
    let prefix = &content.as_bytes()[..(offset as usize).min(content.len())];
    let line = 1 + prefix.iter().filter(|byte| **byte == b'\n').count() as u64;
    let column = prefix
        .iter()
        .rposition(|byte| *byte == b'\n')
        .map_or(prefix.len(), |index| prefix.len() - index - 1) as u64
        + 1;
    (line, column)
}

fn byte_range(path_key: &str, content: &str, location: &crate::parser::ir::Location) -> Value {
    let start = byte_at(content, location.line, location.column);
    let end = byte_at(content, location.end_line, location.end_column).max(start);
    let (start_line, start_column) = position_at(content, start);
    let (end_line, end_column) = position_at(content, end);
    json!({"path_key":path_key,"start_byte":start,"end_byte":end,
        "start_line":start_line,"start_column":start_column,
        "end_line":end_line,"end_column":end_column})
}

pub(crate) fn location_range(
    path_key: &str,
    content: &str,
    location: &crate::parser::ir::Location,
) -> Value {
    byte_range(path_key, content, location)
}

fn graph_state(root: &Path, graph: &Graph, inputs: &[InputRecord]) -> Value {
    let mut entities = Vec::new();
    let mut ids = HashMap::new();
    let mut binding_counts = HashMap::new();
    for (_, symbol) in graph.all_symbols() {
        let base = json!({"path_key":path_key(root, &symbol.location.file),
            "language":language(&symbol.location.file).unwrap_or("python"),
            "qualified_name":symbol.qualified_name,
            "construct_kind":format!("{:?}", symbol.kind).to_lowercase()});
        *binding_counts
            .entry(hash("binding", &base))
            .or_insert(0usize) += 1;
    }
    for (id, symbol) in graph.all_symbols() {
        let key = path_key(root, &symbol.location.file);
        let kind = format!("{:?}", symbol.kind).to_lowercase();
        let base = json!({"path_key":key,"language":language(&symbol.location.file).unwrap_or("python"),
            "qualified_name":symbol.qualified_name,"construct_kind":kind});
        let binding_id = hash("binding", &base);
        if binding_counts.get(&binding_id).copied().unwrap_or(0) != 1 {
            // A lexical identity is ambiguous. The legacy graph retains both
            // observations; the authoritative graph cannot claim either is a
            // unique binding or attach edges to that shared key.
            continue;
        }
        let content = inputs
            .iter()
            .find(|input| path_key(root, &input.path) == key)
            .and_then(|input| input.content.as_deref());
        let Some(content) = content else { continue };
        ids.insert(id, binding_id.clone());
        let mut subject = base;
        subject["binding_id"] = json!(binding_id);
        let range = byte_range(&key, content, &symbol.location);
        entities.push(json!({"key":binding_id,"kind":kind,"subject":subject,"range":range}));
    }
    entities.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
    let mut boundaries = Vec::new();
    let mut dependency_graph = Vec::new();
    let mut flows = Vec::new();
    for (id, _) in graph.all_symbols() {
        let Some(source_key) = ids.get(&id) else {
            continue;
        };
        for edge in graph.edges_from(id) {
            let Some(target_key) = ids.get(&edge.target) else {
                continue;
            };
            let kind = format!("{:?}", edge.kind).to_lowercase();
            let edge_key = hash(
                "binding",
                &json!({"kind":kind,"source_key":source_key,"target_key":target_key}),
            );
            let record = json!({"key":edge_key,"kind":kind,"source_key":source_key,
                "target_key":target_key,"evidence":[]});
            if edge.kind == EdgeKind::Crosses {
                boundaries.push(record.clone());
            }
            dependency_graph.push(record);
            if edge.kind == EdgeKind::Calls {
                let flow_key = hash(
                    "binding",
                    &json!({"kind":"call","node_keys":[source_key,target_key],"edge_keys":[edge_key]}),
                );
                flows.push(
                    json!({"key":flow_key,"kind":"call","node_keys":[source_key,target_key],
                    "edge_keys":[edge_key],"evidence":[]}),
                );
            }
        }
    }
    for rows in [&mut boundaries, &mut dependency_graph, &mut flows] {
        rows.sort_by(|a, b| a["key"].as_str().cmp(&b["key"].as_str()));
        rows.dedup_by(|a, b| a["key"] == b["key"]);
    }
    json!({"entities":entities,"boundaries":boundaries,"dependency_graph":dependency_graph,
        "flows":flows,"type_flows":[]})
}

fn build_finding(
    root: &Path,
    diagnostic: &DiagnosticEntry,
    graph: &Graph,
    inputs: &[InputRecord],
) -> Option<Value> {
    if diagnostic.pattern != "data_dead_end" {
        return None;
    }
    let mut candidates = graph.all_symbols().filter(|(_, symbol)| {
        symbol.qualified_name == diagnostic.entity
            && symbol.kind == SymbolKind::Function
            && symbol.location.file.extension().is_some_and(|e| e == "py")
            && symbol.name.starts_with('_')
            && !symbol.name.starts_with("__")
    });
    let (_, symbol) = candidates.next()?;
    if candidates.next().is_some() {
        return None;
    }
    let key = path_key(root, &symbol.location.file);
    let input = inputs
        .iter()
        .find(|input| path_key(root, &input.path) == key && input.parsed)?;
    let content = input.content.as_deref()?;
    let range = byte_range(&key, content, &symbol.location);
    let subject_base = json!({"path_key":key,"language":"python",
        "qualified_name":symbol.qualified_name,"construct_kind":"function"});
    let binding_id = hash("binding", &subject_base);
    let mut subject = subject_base.clone();
    subject["binding_id"] = json!(binding_id);
    let cause = json!({"key_version":1,"parts":{"produced_binding_id":binding_id,
        "produced_kind":"function","unconsumed_relation":"unconsumed_private_symbol"}});
    let semantic_id = hash(
        "finding.semantic",
        &json!({"pattern_id":"data_dead_end","subject":subject_base,"cause":cause}),
    );
    let evidence: Vec<Value> = diagnostic
        .evidence
        .iter()
        .map(|e| {
            json!({"kind":"graph_inbound_consumption",
        "observation":e.observation,"range":Value::Null})
        })
        .collect();
    let observation_id = hash(
        "finding.observation",
        &json!({"semantic_id":semantic_id,
        "source_sha256":digest(content.as_bytes()),"range":range,"evidence":evidence}),
    );
    Some(json!({"identity_status":"stable","semantic_id":semantic_id,
        "observation_id":observation_id,"pattern_id":"data_dead_end",
        "severity":diagnostic.severity,"confidence":diagnostic.confidence,
        "subject":subject,"cause":cause,"range":range,"evidence":evidence,
        "suggestion":diagnostic.suggestion}))
}

const FIELDS: [&str; 14] = [
    "schema_version",
    "producer_contract",
    "producer",
    "execution",
    "outcome",
    "findings_present",
    "gate",
    "findings",
    "scope",
    "provenance",
    "inventory",
    "losses",
    "pattern_state",
    "analysis_graph",
];

pub(crate) fn refresh_identity(authority: &mut Value) {
    let body: serde_json::Map<String, Value> = FIELDS
        .iter()
        .map(|key| ((*key).to_string(), authority[*key].clone()))
        .collect();
    authority["report_identity"] = json!(hash("report", &Value::Object(body)));
}

pub(crate) fn set_gate(authority: &mut Value, severity: &str, confidence: &str) {
    authority["scope"]["gate_threshold"] =
        json!({"min_severity":severity,"min_confidence":confidence});
    authority["provenance"]["config"]["effective"]["gate_threshold"] =
        authority["scope"]["gate_threshold"].clone();
    let effective = authority["provenance"]["config"]["effective"].clone();
    authority["provenance"]["config"]["effective_digest"] = json!(hash("config", &effective));
    let scope_body = authority["scope"]
        .as_object()
        .expect("scope object")
        .iter()
        .filter(|(k, _)| k.as_str() != "digest")
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    authority["scope"]["digest"] = json!(hash("scope", &Value::Object(scope_body)));
    let qualifies = authority["findings"]["all"]
        .as_array()
        .expect("findings array")
        .iter()
        .filter(|finding| {
            let s = match finding["severity"].as_str().unwrap_or("") {
                "critical" => 2,
                "warning" => 1,
                _ => 0,
            };
            let c = match finding["confidence"].as_str().unwrap_or("") {
                "high" => 2,
                "moderate" => 1,
                _ => 0,
            };
            let min_s = match severity {
                "critical" => 2,
                "warning" => 1,
                _ => 0,
            };
            let min_c = match confidence {
                "high" => 2,
                "moderate" => 1,
                _ => 0,
            };
            s >= min_s && c >= min_c
        })
        .count();
    authority["gate"] = if authority["execution"] == "incomplete" {
        json!({"verdict":"not_evaluated","qualifying_count":qualifies})
    } else {
        json!({"verdict":if qualifies>0 {"fail"} else {"pass"},"qualifying_count":qualifies})
    };
    refresh_identity(authority);
}

pub(crate) fn build(
    root: &Path,
    config: &Config,
    languages: &[String],
    inputs: &[InputRecord],
    diagnostics: &[DiagnosticEntry],
    graph: &Graph,
    mut losses: Vec<Value>,
) -> Value {
    let blockers = consumer_closure_blockers(inputs, root, &losses);
    let mut findings: Vec<Value> = Vec::new();
    for diagnostic in diagnostics {
        if let Some(finding) = build_finding(root, diagnostic, graph, inputs) {
            if blockers.is_empty() {
                findings.push(finding);
            } else {
                // Project-level bindings require a closed selected consumer
                // universe. Missing source/resolution cannot prove zero uses.
                losses.push(consumer_closure_loss(&finding, &blockers));
            }
        }
    }
    findings.sort_by(|a, b| a["semantic_id"].as_str().cmp(&b["semantic_id"].as_str()));
    // Legacy findings without an implemented cause proof are visible as debt,
    // never promoted into the authoritative finding catalog.
    if diagnostics.len() != findings.len() {
        losses.push(loss(
            "semantic_proof_unavailable",
            String::new(),
            None,
            "finding",
        ));
    }
    let mut by_pattern: BTreeMap<&str, u64> = PATTERNS.iter().map(|p| (*p, 0)).collect();
    let mut by_severity: BTreeMap<&str, u64> = ["critical", "warning", "info"]
        .into_iter()
        .map(|s| (s, 0))
        .collect();
    for finding in &findings {
        if let Some(v) = by_pattern.get_mut(finding["pattern_id"].as_str().unwrap_or("")) {
            *v += 1;
        }
        if let Some(v) = by_severity.get_mut(finding["severity"].as_str().unwrap_or("")) {
            *v += 1;
        }
    }
    let mut entries = Vec::new();
    let mut provenance_inputs = Vec::new();
    let mut counts: HashMap<&str, u64> = HashMap::new();
    for input in inputs {
        let key = path_key(root, &input.path);
        *counts.entry(input.disposition).or_default() += 1;
        if input.read {
            *counts.entry("read").or_default() += 1;
        }
        if input.parsed {
            *counts.entry("parsed").or_default() += 1;
        }
        if input.rejected_reason.is_some() {
            *counts.entry("rejected").or_default() += 1;
        }
        if input.changed_during_run {
            *counts.entry("changed_during_run").or_default() += 1;
        }
        entries.push(json!({"path_key":key,"display_path":input.path.strip_prefix(root).ok().map(|p|p.to_string_lossy().to_string()),
            "disposition":input.disposition,"reason":input.reason,"language":input.language,
            "read":input.read,"parsed":input.parsed,"rejected_reason":input.rejected_reason,
            "changed_during_run":input.changed_during_run}));
        provenance_inputs.push(json!({"path_key":key,"disposition":input.disposition,
            "raw_sha256":input.content.as_ref().map(|s|digest(s.as_bytes())),
            "bytes_consumed":input.content.as_ref().map(|s|s.len())}));
    }
    entries.sort_by(|a, b| a["path_key"].as_str().cmp(&b["path_key"].as_str()));
    provenance_inputs.sort_by(|a, b| a["path_key"].as_str().cmp(&b["path_key"].as_str()));
    let inventory = json!({"discovered":inputs.len(),"policy_excluded":counts.get("policy_excluded").copied().unwrap_or(0),
        "unsupported":counts.get("unsupported").copied().unwrap_or(0),"selected":counts.get("selected").copied().unwrap_or(0),
        "read":counts.get("read").copied().unwrap_or(0),"parsed":counts.get("parsed").copied().unwrap_or(0),
        "rejected":counts.get("rejected").copied().unwrap_or(0),
        "changed_during_run":counts.get("changed_during_run").copied().unwrap_or(0),"entries":entries});
    let origin = if config.origin.is_empty() {
        "default"
    } else {
        config.origin.as_str()
    };
    let config_key = config
        .config_path
        .as_ref()
        .and_then(|p| p.strip_prefix(root).ok())
        .map(|p| path_key(root, &root.join(p)));
    let mut controls = Vec::new();
    if let (Some(key), Some(digest)) = (&config_key, &config.source_sha256) {
        controls.push(
            json!({"path_key":key,"kind":"effective_config","origin":origin,"raw_sha256":digest}),
        );
    }
    let effective = json!({"languages":languages,"exclude":config.exclude,
        "gate_threshold":{"min_severity":"critical","min_confidence":"low"}});
    let config_provenance = json!({"origin":origin,"source_path_key":config_key,
        "source_sha256":config.source_sha256,"effective_digest":hash("config",&effective),"effective":effective});
    let effective_languages: Vec<String> = inputs
        .iter()
        .filter(|i| i.disposition == "selected")
        .filter_map(|i| i.language.clone())
        .collect();
    let mut effective_languages = effective_languages;
    effective_languages.sort();
    effective_languages.dedup();
    let mut scope = json!({"root_kind":"directory","path_platform":"linux-posix-bytes-v1",
        "declared_languages":config.languages,"effective_languages":effective_languages,
        "include_paths":[],"exclusions":config.exclude.iter().map(|p|json!({"pattern":p,"origin":origin})).collect::<Vec<_>>(),
        "pattern_ids":PATTERNS,"gate_threshold":{"min_severity":"critical","min_confidence":"low"},
        "policy_origin":origin});
    scope["digest"] = json!(hash("scope", &scope));
    let adapters:Vec<Value>=scope["effective_languages"].as_array().unwrap().iter().filter_map(|v|v.as_str()).map(|lang|{
        let grammar=match lang {"python"=>"tree-sitter-python/0.25","rust"=>"tree-sitter-rust/0.24",_=>"tree-sitter-javascript/0.25"};
        json!({"language":lang,"adapter_version":env!("CARGO_PKG_VERSION"),"grammar_version":grammar,
            "capabilities_digest":hash("capabilities",&json!({"language":lang,"implemented":["symbols","calls","imports"]}))})
    }).collect();
    let registry =
        json!({"patterns":PATTERNS,"languages":["python","javascript","typescript","rust"]});
    let build_sha = option_env!("FLOWSPEC_BUILD_SHA").unwrap_or("unbound");
    let tree_sha = option_env!("FLOWSPEC_SOURCE_TREE_SHA").unwrap_or("unbound");
    let producer = json!({"tool_version":env!("CARGO_PKG_VERSION"),"build_sha":build_sha,
        "source_tree_sha":tree_sha,"report_abi":"v1","generated_by":"analyzer"});
    let provenance = json!({"config":config_provenance,"registry":{"digest":hash("registry",&registry)},
        "adapters":adapters,"controls":controls,"inputs":provenance_inputs,
        "cache":{"requested_mode":"full","executed_mode":"full","rebuild_reason":Value::Null},
        "build_receipt":hash("build",&producer)});
    // Colliding lexical bindings have no unique stable graph key. Keep the
    // source observations, but make the missing authoritative join explicit.
    let mut binding_counts = HashMap::new();
    for (_, symbol) in graph.all_symbols() {
        let base = json!({"path_key":path_key(root, &symbol.location.file),
            "language":language(&symbol.location.file).unwrap_or("python"),
            "qualified_name":symbol.qualified_name,
            "construct_kind":format!("{:?}", symbol.kind).to_lowercase()});
        *binding_counts
            .entry(hash("binding", &base))
            .or_insert(0usize) += 1;
    }
    if binding_counts.values().any(|count| *count > 1) {
        losses.push(loss("identity_uncertain", String::new(), None, "binding"));
    }
    let mut pattern_state = Vec::new();
    for pattern in PATTERNS {
        let cells:Vec<Value>=scope["effective_languages"].as_array().unwrap().iter().map(|lang|{
            let mut reasons:Vec<String>=losses.iter().filter_map(|loss|loss["reason"].as_str().map(str::to_string)).collect();
            let capabilities: &[&str] = match pattern {
                "data_dead_end" => &["symbols", "calls", "value_references", "exports"],
                "phantom_dependency" | "missing_reexport" => &["imports", "exports", "value_references"],
                "circular_dependency" | "layer_violation" => &["imports", "dependency_edges"],
                "duplication" => &["symbols", "binding_structure"],
                "contract_mismatch" | "partial_wiring" | "asymmetric_handling" => &["symbols", "calls", "control_flow"],
                "stale_reference" | "incomplete_migration" => &["imports", "resolution"],
                _ => &["symbols", "calls", "resolution"],
            };
            if pattern == "layer_violation" { reasons.push("policy_absent".to_string()); }
            let state=if pattern=="layer_violation" {"not_applicable"} else if reasons.is_empty(){"complete"}else{"incomplete"};
            json!({"language":lang,"state":state,"required_capabilities":capabilities,"affected_paths":[],"reason_codes":reasons})
        }).collect();
        pattern_state.push(json!({"pattern_id":pattern,"cells":cells}));
    }
    let execution = if losses.is_empty() {
        "complete"
    } else {
        "incomplete"
    };
    let outcome = if execution == "incomplete" {
        "incomplete"
    } else if findings.is_empty() {
        "clean"
    } else {
        "findings"
    };
    let graph_state = graph_state(root, graph, inputs);
    let mut authority = json!({"schema_version":2,"producer_contract":"flowspec.analysis.provenance.v1",
        "producer":producer,"execution":execution,"outcome":outcome,"findings_present":!findings.is_empty(),
        "gate":{"verdict":"pass","qualifying_count":0},
        "findings":{"total":findings.len(),"by_pattern":by_pattern,"by_severity":by_severity,"all":findings},
        "scope":scope,"provenance":provenance,"inventory":inventory,"losses":losses,
        "pattern_state":pattern_state,"analysis_graph":graph_state,
        "display":{"filters":{"checks":[],"min_severity":Value::Null,"min_confidence":Value::Null},"indices":[]},
        "continuation":{"shown":0,"matched_total":0,"full":0,"truncated":false,"next_cursor":Value::Null}});
    set_gate(&mut authority, "critical", "low");
    authority
}

pub(crate) fn language_for(path: &Path) -> Option<String> {
    language(path).map(str::to_string)
}
