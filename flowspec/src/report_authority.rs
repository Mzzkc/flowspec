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

pub(crate) use crate::registry::PATTERN_IDS as PATTERNS;

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

pub(crate) fn hash(domain: &str, value: &Value) -> String {
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

/// Reduce the selected adapter's actual per-file evidence without reparsing through a default registry.
pub(crate) fn parser_losses(
    root: &Path,
    path: &Path,
    source: &str,
    descriptor: &crate::registry::LanguageDescriptor,
    registry: &crate::registry::Registry,
    report: &crate::parser::report::ParseReport,
) -> Vec<Value> {
    use crate::parser::report::{Capability, CapabilityDisposition, ParseEventCode};
    let key = path_key(root, path);
    let provenance_valid = report.file == path
        && report.source_bytes == source.len() as u64
        && report.source_sha256 == format!("{:x}", Sha256::digest(source.as_bytes()))
        && report.language == descriptor.id
        && report.grammar_version == descriptor.grammar_version
        && report.adapter_version == descriptor.adapter_version
        && report.schema_version == 1;
    let mut losses = Vec::new();
    for pattern in registry.patterns() {
        let missing: Vec<_> = pattern
            .required_capabilities
            .iter()
            .filter(|name| {
                !provenance_valid
                    || !descriptor.capabilities.iter().any(|c| c == **name)
                    || !Capability::ALL
                        .iter()
                        .find(|c| c.as_str() == **name)
                        .is_some_and(|c| {
                            report.capabilities.get(c) == Some(&CapabilityDisposition::Implemented)
                        })
            })
            .copied()
            .collect();
        if !missing.is_empty() {
            let mut item = loss(
                "semantic_proof_unavailable",
                key.clone(),
                None,
                "adapter_capability",
            );
            item["affected_patterns"] = json!([pattern.id]);
            item["required_capabilities"] = json!(missing);
            item["id"] = json!(hash(
                "parser-capability-loss",
                &json!({"path":key,"pattern":pattern.id,"missing":missing,"schema":report.schema_version})
            ));
            losses.push(item);
        }
    }
    for event in &report.events {
        let reason = match event.code {
            ParseEventCode::ParserRecovery | ParseEventCode::MissingNode => "parse_recovery",
            ParseEventCode::DepthTruncated => "depth_truncation",
            ParseEventCode::UnsupportedDynamic | ParseEventCode::UnsupportedSyntax => {
                "unsupported_construct"
            }
            ParseEventCode::CapabilityUnavailable => "capability_unavailable",
            ParseEventCode::UnresolvedReference => "unresolved_reference",
            ParseEventCode::UnknownLegacyReport => "semantic_proof_unavailable",
        };
        let affects: Vec<_> = event.affects.iter().map(|c| c.as_str()).collect();
        let patterns: Vec<_> = registry
            .patterns()
            .iter()
            .filter(|p| p.required_capabilities.iter().any(|c| affects.contains(c)))
            .map(|p| p.id)
            .collect();
        let range = offset_range(&key, source, event.byte_start, event.byte_end);
        let mut item = loss(
            reason,
            key.clone(),
            Some(range),
            if event.code == ParseEventCode::UnknownLegacyReport {
                "adapter_recovery"
            } else {
                "parse"
            },
        );
        item["required_capabilities"] = json!(affects);
        item["affected_patterns"] = json!(patterns);
        item["id"] = json!(hash(
            "parser-event-loss",
            &json!({"path":key,"code":event.code,"start":event.byte_start,"end":event.byte_end,"affects":affects})
        ));
        losses.push(item);
    }
    losses
}

fn loss_applies_to_language(
    loss: &Value,
    language: &Value,
    inputs: &[InputRecord],
    root: &Path,
) -> bool {
    loss["scope_paths"].as_array().is_none_or(|paths| {
        paths.is_empty()
            || paths.iter().any(|key| {
                key == ""
                    || inputs.iter().any(|input| {
                        input.language.as_deref() == language.as_str()
                            && key.as_str() == Some(path_key(root, &input.path).as_str())
                    })
            })
    })
}

fn consumer_closure_blockers(inputs: &[InputRecord], root: &Path, losses: &[Value]) -> Vec<String> {
    let mut blocked = Vec::new();
    for loss in losses {
        let reason = loss["reason"].as_str().unwrap_or("");
        // Type-only and control-flow debt cannot hide a runtime consumer. Explicit
        // structured capability debt blocks closure only when that evidence is lost.
        if let Some(caps) = loss["required_capabilities"].as_array() {
            if !caps.is_empty() && !caps.iter().any(|c| c == "consumer_closure") {
                continue;
            }
        }
        if loss["origin"] == "Annotation" {
            continue;
        }
        if !matches!(
            reason,
            "semantic_proof_unavailable"
                | "discovery_error"
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

#[cfg(feature = "fault_probe")]
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

pub(crate) fn offset_range(path_key: &str, content: &str, start: u64, end: u64) -> Value {
    let start = start.min(content.len() as u64);
    let end = end.min(content.len() as u64).max(start);
    let (start_line, start_column) = position_at(content, start);
    let (end_line, end_column) = position_at(content, end);
    json!({"path_key":path_key,"start_byte":start,"end_byte":end,
        "start_line":start_line,"start_column":start_column,
        "end_line":end_line,"end_column":end_column})
}

fn byte_range(path_key: &str, content: &str, location: &crate::parser::ir::Location) -> Value {
    offset_range(
        path_key,
        content,
        byte_at(content, location.line, location.column),
        byte_at(content, location.end_line, location.end_column),
    )
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
            "language":input_language(inputs, &symbol.location.file),
            "qualified_name":symbol.qualified_name,
            "construct_kind":format!("{:?}", symbol.kind).to_lowercase()});
        *binding_counts
            .entry(hash("binding", &base))
            .or_insert(0usize) += 1;
    }
    for (id, symbol) in graph.all_symbols() {
        let key = path_key(root, &symbol.location.file);
        let kind = format!("{:?}", symbol.kind).to_lowercase();
        let base = json!({"path_key":key,"language":input_language(inputs, &symbol.location.file),
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
    let mut candidates = graph.all_symbols().filter(|(id, symbol)| {
        symbol.qualified_name == diagnostic.entity
            && symbol.kind == SymbolKind::Function
            && ((input_language(inputs, &symbol.location.file) == "javascript"
                || input_language(inputs, &symbol.location.file) == "typescript")
                && symbol.visibility == crate::parser::ir::Visibility::Private
                || (symbol.location.file.extension().is_some_and(|e| e == "py")
                    && symbol.name.starts_with('_')
                    && !symbol.name.starts_with("__"))
                || (symbol.location.file.extension().is_some_and(|e| e == "rs")
                    && graph.effective_visibility(*id)
                        == Some(crate::parser::ir::Visibility::Private)
                    && graph
                        .semantic_files()
                        .iter()
                        .find(|f| f.path == symbol.location.file)
                        .and_then(|f| {
                            f.executables
                                .iter()
                                .find(|p| p.qualified_name == symbol.qualified_name)
                        })
                        .is_some_and(|p| p.closure && p.consumers.is_empty())))
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
    let subject_base = json!({"path_key":key,"language":input_language(inputs,&symbol.location.file),
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

/// Effective source and registry scope shared by every report consumer.
pub(crate) struct ReportContext<'a> {
    pub root: &'a Path,
    pub config: &'a Config,
    pub languages: &'a [String],
    pub registry: &'a crate::registry::Registry,
}

pub(crate) fn build(
    context: ReportContext<'_>,
    inputs: &[InputRecord],
    diagnostics: &[DiagnosticEntry],
    graph: &Graph,
    mut losses: Vec<Value>,
) -> Value {
    let ReportContext {
        root,
        config,
        languages,
        registry,
    } = context;
    if let Some(policy) = &config.semantic_policy {
        for (pattern, requested, operation) in [
            (
                "partial_wiring",
                !policy.peers.is_empty(),
                "peer operation/complete CFG closure",
            ),
            (
                "asymmetric_handling",
                !policy.peers.is_empty(),
                "peer effect/resource invariants",
            ),
            (
                "incomplete_migration",
                !policy.migrations.is_empty(),
                "old/new authority consumer closure",
            ),
            (
                "missing_reexport",
                !policy.exports.is_empty(),
                "complete parent export graph",
            ),
            (
                "layer_violation",
                !policy.layers.is_empty(),
                "resolved forbidden module edges",
            ),
        ] {
            if requested {
                let mut item = loss(
                    "policy_operation_unavailable",
                    String::new(),
                    None,
                    "semantic_policy",
                );
                item["affected_patterns"] = json!([pattern]);
                item["required_capabilities"] = json!([operation]);
                item["policy"] = json!(policy);
                losses.push(item);
            }
        }
    }
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
    append_semantic_proofs(root, inputs, graph, &mut findings, &mut losses);
    append_duplication_proofs(root, config, inputs, graph, &mut findings, &mut losses);
    coalesce_semantic_losses(&mut losses);
    findings.sort_by(|a, b| a["semantic_id"].as_str().cmp(&b["semantic_id"].as_str()));
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
        "gate_threshold":{"min_severity":"critical","min_confidence":"low"},"semantic_policy":config.semantic_policy});
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
    let adapters: Vec<Value> = registry.languages().iter().filter(|d|languages.contains(&d.id)).map(|d|json!({"language":d.id,"adapter_version":d.adapter_version,"grammar_version":d.grammar_version,"capabilities_digest":hash("capabilities",&json!({"language":d.id,"implemented":d.capabilities}))})).collect();
    let mut registry_metadata = registry.report_metadata();
    registry_metadata["digest"] = json!(registry.digest());
    let build_sha = option_env!("FLOWSPEC_BUILD_SHA").unwrap_or("unbound");
    let tree_sha = option_env!("FLOWSPEC_SOURCE_TREE_SHA").unwrap_or("unbound");
    let producer = json!({"tool_version":env!("CARGO_PKG_VERSION"),"build_sha":build_sha,
        "source_tree_sha":tree_sha,"report_abi":"v1","generated_by":"analyzer"});
    let provenance = json!({"config":config_provenance,"registry":registry_metadata,
        "adapters":adapters,"controls":controls,"inputs":provenance_inputs,
        "cache":{"requested_mode":"full","executed_mode":"full","rebuild_reason":Value::Null},
        "build_receipt":hash("build",&producer)});
    // Colliding lexical bindings have no unique stable graph key. Keep the
    // source observations, but make the missing authoritative join explicit.
    let mut binding_counts = HashMap::new();
    for (_, symbol) in graph.all_symbols() {
        let base = json!({"path_key":path_key(root, &symbol.location.file),
            "language":input_language(inputs, &symbol.location.file),
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
            let mut reasons:Vec<String>=losses.iter().filter(|loss| loss_applies_to_language(loss,lang,inputs,root)).filter(|loss| loss["affected_patterns"].as_array().is_some_and(|patterns| patterns.iter().any(|p| p == pattern))).filter_map(|loss|loss["reason"].as_str().map(str::to_string)).collect();
            reasons.sort(); reasons.dedup();
            let capabilities = registry.patterns().iter().find(|d|d.id==pattern).expect("canonical pattern").required_capabilities;
            let relevant_losses: Vec<_> = losses.iter().filter(|loss| loss_applies_to_language(loss,lang,inputs,root)).filter(|loss| loss["affected_patterns"].as_array().is_some_and(|patterns| patterns.iter().any(|p| p == pattern))).collect();
            let mut capabilities: std::collections::BTreeSet<String> = capabilities.iter().map(|name| name.to_string()).collect();
            let mut affected_paths = std::collections::BTreeSet::new();
            for loss in relevant_losses {
                for capability in loss["required_capabilities"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    capabilities.insert(capability.to_string());
                }
                for key in loss["scope_paths"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    if key.is_empty() {
                        affected_paths.extend(inputs.iter().filter(|input| input.disposition == "selected").map(|input| path_key(root, &input.path)));
                    } else { affected_paths.insert(key.to_string()); }
                }
            }
            let policy_absent = pattern=="layer_violation" && config.semantic_policy.as_ref().is_none_or(|policy|policy.layers.is_empty());
            if policy_absent { reasons.push("policy_absent".to_string()); }
            let state=if policy_absent {"not_applicable"} else if reasons.is_empty(){"complete"}else{"incomplete"};
            json!({"language":lang,"state":state,"required_capabilities":capabilities,"affected_paths":affected_paths,"reason_codes":reasons})
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

fn input_language<'a>(inputs: &'a [InputRecord], path: &Path) -> &'a str {
    inputs
        .iter()
        .find(|input| input.path == path)
        .and_then(|input| input.language.as_deref())
        .unwrap_or("unknown")
}

fn append_semantic_proofs(
    root: &Path,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    for file in graph.semantic_files() {
        let key = path_key(root, &file.path);
        let Some(input) = inputs
            .iter()
            .find(|input| input.path == file.path && input.parsed && !input.changed_during_run)
        else {
            continue;
        };
        let Some(content) = input.content.as_deref() else {
            continue;
        };
        for issue in &file.issues {
            let mut item = loss(
                &issue.reason,
                key.clone(),
                Some(offset_range(
                    &key,
                    content,
                    issue.range.0 as u64,
                    issue.range.1 as u64,
                )),
                "semantic_prerequisite",
            );
            item["affected_patterns"] = json!([issue.pattern]);
            item["required_capabilities"] = json!([issue.operation]);
            item["operation"] = json!(issue.operation);
            losses.push(item);
        }
        for value in &file.values {
            if !value.closure
                || !value.reads.is_empty()
                || !value.writes.is_empty()
                || value.produced_type.is_empty()
            {
                continue;
            }
            let qualified_name = format!(
                "{}::{}::{}@scope{}",
                file.path.file_name().unwrap_or_default().to_string_lossy(),
                value.owner,
                value.name,
                value.scope
            );
            let base = json!({"path_key":key,"language":input.language,"qualified_name":qualified_name,"construct_kind":"local_value","scope_ordinal":value.scope,"binding_ordinal":value.ordinal});
            let binding_id = hash("binding", &base);
            let mut subject = base.clone();
            subject["binding_id"] = json!(binding_id);
            subject["name"] = json!(value.name);
            let cause = json!({"key_version":1,"parts":{"produced_binding_id":binding_id,"produced_kind":"local_value","produced_type":value.produced_type,"unconsumed_relation":"unconsumed_private_value"}});
            let semantic_id = hash(
                "finding.semantic",
                &json!({"pattern_id":"data_dead_end","subject":base,"cause":cause}),
            );
            let range = offset_range(&key, content, value.range.0 as u64, value.range.1 as u64);
            let evidence = json!([{"kind":"binding_value_consumer_closure","consumer_closure":"complete","binding_id":binding_id,"initializer_range":offset_range(&key,content,value.initializer.0 as u64,value.initializer.1 as u64),"effect_protocol":value.protocol,"reads":value.reads,"writes":value.writes,"escape":"none","visibility":"function_local"}]);
            let observation_id = hash(
                "finding.observation",
                &json!({"semantic_id":semantic_id,"source_sha256":digest(content.as_bytes()),"range":range,"evidence":evidence}),
            );
            findings.push(json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":"data_dead_end","severity":"warning","confidence":"high","subject":subject,"cause":cause,"range":range,"evidence":evidence,"suggestion":"Remove the unused local value or connect it to its intended consumer."}));
        }
    }
}

fn append_duplication_proofs(
    root: &Path,
    config: &Config,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    use crate::parser::normalization::{RULE_SHA256, RULE_VERSION};
    let mut supported = Vec::new();
    for file in graph.semantic_files() {
        let key = path_key(root, &file.path);
        let Some(input) = inputs
            .iter()
            .find(|i| i.path == file.path && i.parsed && !i.changed_during_run)
        else {
            continue;
        };
        let Some(content) = input.content.as_deref() else {
            continue;
        };
        let language = input.language.as_deref().unwrap_or("unknown");
        for function in &file.functions {
            for issue in &function.issues {
                let mut item = loss(
                    &issue.reason,
                    key.clone(),
                    Some(offset_range(
                        &key,
                        content,
                        if issue.reason == "DUP_TYPE_UNRESOLVED" {
                            0
                        } else {
                            issue.range.0 as u64
                        },
                        if issue.reason == "DUP_TYPE_UNRESOLVED" {
                            content.len() as u64
                        } else {
                            issue.range.1 as u64
                        },
                    )),
                    "duplication_normalization",
                );
                item["affected_patterns"] = json!(["duplication"]);
                item["required_capabilities"] = json!([issue.operation]);
                losses.push(item);
            }
            if function.normal_form.is_none() {
                continue;
            }
            let environment = config
                .semantic_policy
                .as_ref()
                .and_then(|p| p.environments.get(language));
            let known_environment = environment.is_some_and(|e| match language {
                "javascript" => e.version == "ECMA-262/2025" && e.target == "ECMAScript",
                "python" => e.version == "Python/3.14" && e.target == "Python",
                // These protocols have no complete type/ownership normalizer yet.
                "typescript" | "rust" => false,
                _ => false,
            });
            if !known_environment {
                let mut item = loss(
                    "DUP_ENVIRONMENT_UNRESOLVED",
                    key.clone(),
                    Some(offset_range(
                        &key,
                        content,
                        function.range.0 as u64,
                        function.range.1 as u64,
                    )),
                    "duplication_environment",
                );
                item["affected_patterns"] = json!(["duplication"]);
                item["required_capabilities"] =
                    json!(["explicit language/runtime/target environment"]);
                losses.push(item);
                continue;
            }
            if let Some(normal) = &function.normal_form {
                if function.weight >= 8
                    && ((function.control_shape && function.operations >= 3)
                        || function.max_operation_chain >= 3)
                {
                    let env = environment.unwrap();
                    let mut normal = normal.clone();
                    normal[1][1][4] = json!([language, env.version, env.target, env.overflow]);
                    supported.push((file, input, content, function, normal));
                }
            }
        }
    }
    for i in 0..supported.len() {
        for j in i + 1..supported.len() {
            let (file_a, input_a, content_a, a, normal_a) = &supported[i];
            let (file_b, input_b, content_b, b, normal_b) = &supported[j];
            if input_a.language != input_b.language || normal_a != normal_b {
                continue;
            }
            let locator =
                |file: &crate::parser::semantic::SemanticFile,
                 f: &crate::parser::normalization::NormalizedFunction| {
                    json!([
                        input_a.language,
                        file.path
                            .strip_prefix(root)
                            .unwrap_or(&file.path)
                            .to_string_lossy()
                            .replace('\\', "/"),
                        f.declaration_path,
                        f.declaration_ordinal
                    ])
                };
            let mut locators = [locator(file_a, a), locator(file_b, b)];
            locators.sort_by_key(crate::parser::normalization::canonical_bytes);
            let identity = json!([
                "duplication",
                RULE_VERSION,
                "binding-aware-whole-body-equivalence",
                locators[0],
                locators[1]
            ]);
            let semantic_id = format!(
                "duplication:{:x}",
                Sha256::digest(crate::parser::normalization::canonical_bytes(&identity))
            );
            let key_a = path_key(root, &file_a.path);
            let key_b = path_key(root, &file_b.path);
            let range_a = offset_range(&key_a, content_a, a.range.0 as u64, a.range.1 as u64);
            let range_b = offset_range(&key_b, content_b, b.range.0 as u64, b.range.1 as u64);
            let evidence = json!([{"rule_version":RULE_VERSION,"rule_sha256":RULE_SHA256,"relation":"binding-aware-whole-body-equivalence","weight":a.weight,"operations":a.operations,"control_shape":a.control_shape,"max_operation_chain":a.max_operation_chain,"normal_form":normal_a,"normal_form_digest":digest(&crate::parser::normalization::canonical_bytes(normal_a)),"binder_bijection":a.binder_names.iter().zip(&b.binder_names).collect::<Vec<_>>(),"ignored_ranges":[a.ignored,b.ignored],"ranges":[range_a,range_b],"body_ranges":[offset_range(&key_a,content_a,a.body_range.0 as u64,a.body_range.1 as u64),offset_range(&key_b,content_b,b.body_range.0 as u64,b.body_range.1 as u64)],"input_digests":[digest(content_a.as_bytes()),digest(content_b.as_bytes())]}]);
            let observation_id = hash(
                "finding.observation",
                &json!({"semantic_id":semantic_id,"evidence":evidence}),
            );
            findings.push(json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":"duplication","severity":"warning","confidence":"high","subject":{"path_key":key_a,"language":input_a.language,"qualified_name":a.name,"construct_kind":"function_pair","locators":locators},"cause":{"key_version":1,"parts":{"relation":"binding-aware-whole-body-equivalence","rule_version":RULE_VERSION,"function_locators":locators}},"range":range_a,"evidence":evidence,"suggestion":"Consider consolidating the exactly equivalent function bodies while preserving their public contracts."}));
        }
    }
}

/// Sharing event metadata preserves every range and affected path while keeping
/// bounded ordinary reports useful. Parser/fault records keep their exact form.
fn coalesce_semantic_losses(losses: &mut Vec<Value>) {
    let mut groups: BTreeMap<String, Value> = BTreeMap::new();
    let mut retained = Vec::new();
    for item in losses.drain(..) {
        if !matches!(
            item["stage"].as_str(),
            Some("semantic_prerequisite" | "duplication_normalization" | "duplication_environment" | "adapter_capability" | "resolution" | "parse")
        ) {
            retained.push(item);
            continue;
        }
        let key = serde_json::to_string(&json!([
            item["stage"],
            item["reason"],
            item["affected_patterns"],
            item["required_capabilities"], item["origin"]
        ]))
        .unwrap();
        if let Some(group) = groups.get_mut(&key) {
            for field in ["scope_paths", "ranges"] {
                let items = item[field].as_array().unwrap();
                let entries = group[field].as_array_mut().unwrap();
                for value in items {
                    if !entries.contains(value) {
                        entries.push(value.clone());
                    }
                }
            }
        } else {
            groups.insert(key, item);
        }
    }
    for mut group in groups.into_values() {
        // These new parser semantic prerequisites use exact half-open UTF-8
        // ranges; repeated derived line/column coordinates are unnecessary.
        // Existing parser/fault records retain their original schema verbatim.
        if matches!(group["stage"].as_str(), Some("semantic_prerequisite" | "duplication_normalization" | "duplication_environment")) {
        if let Some(ranges) = group["ranges"].as_array_mut() {
            for range in ranges {
                *range = json!({"path_key":range["path_key"],"start_byte":range["start_byte"],"end_byte":range["end_byte"]});
            }
        }
        }
        group["scope_paths"]
            .as_array_mut()
            .unwrap()
            .sort_by_key(|v| v.as_str().unwrap_or("").to_string());
        group["id"] = json!(hash(
            "loss",
            &json!([
                group["stage"],
                group["reason"],
                group["scope_paths"],
                group["ranges"],
                group["required_capabilities"]
            ])
        ));
        retained.push(group);
    }
    *losses = retained;
}
