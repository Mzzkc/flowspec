//! User-authored, binding-exact invocation contracts. This consumer proves the
//! declared normal-exit relation only; all-exit/resource policies remain debt.
use crate::report_authority::{
    digest, hash, location_range, loss, offset_range, path_key, InputRecord,
};
use crate::{
    config::Config,
    graph::Graph,
    parser::ir::{EdgeKind, SymbolId, SymbolKind},
};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::Path;
pub(super) fn resolve(graph: &Graph, root: &Path, authority: &str) -> Option<SymbolId> {
    let (path, name) = authority.split_once("::")?;
    if name.contains("::") {
        return None;
    }
    let mut matches = graph.all_symbols().filter(|(_, s)| {
        s.location
            .file
            .strip_prefix(root)
            .is_ok_and(|p| p.to_string_lossy() == path)
            && s.kind == SymbolKind::Function
            && graph
                .semantic_files()
                .iter()
                .find(|f| f.path == s.location.file)
                .is_some_and(|f| f.normal_control.iter().any(|c| c.name == name))
            && (s.qualified_name
                == format!(
                    "{}::{name}",
                    s.location
                        .file
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                )
                || s.name == name)
    });
    match (matches.next(), matches.next()) {
        (Some((id, _)), None) => Some(id),
        _ => None,
    }
}
fn invocation_paths(
    id: SymbolId,
    graph: &Graph,
    inputs: &[InputRecord],
    root: &Path,
    active: &mut HashSet<SymbolId>,
    all_exits: bool,
) -> Option<Vec<HashSet<SymbolId>>> {
    if active.len() > 32 || !active.insert(id) {
        return None;
    }
    let symbol = graph.get_symbol(id)?;
    let input = inputs
        .iter()
        .find(|i| i.path == symbol.location.file && i.parsed && !i.changed_during_run)?;
    let content = input.content.as_deref()?;
    let key = path_key(root, &input.path);
    let range = location_range(&key, content, &symbol.location);
    let start = range["start_byte"].as_u64()? as usize;
    let file = graph
        .semantic_files()
        .iter()
        .find(|f| f.path == input.path)?;
    let control = file
        .normal_control
        .iter()
        .find(|c| c.name == symbol.name && c.range.0 == start)?;
    if !control.unresolved.is_empty() || (all_exits && active.len() > 1 && control.exceptional_paths > 0) {
        return None;
    }
    let mut result = Vec::new();
    for path in if all_exits { &control.all_paths } else { &control.paths } {
        let mut accumulated = vec![HashSet::new()];
        for &(a, b) in path {
            let mut targets = graph
                .edges_from(id)
                .iter()
                .filter(|e| {
                    e.kind == EdgeKind::Calls
                        && graph.get_symbol(e.target).is_some_and(|s| {
                            s.kind == SymbolKind::Function
                                && !s.annotations.iter().any(|a| a == "import")
                        })
                })
                .filter_map(|e| {
                    let r = graph.get_reference(e.reference_id?)?;
                    let rr = location_range(&key, content, &r.location);
                    (rr["start_byte"] == a && rr["end_byte"] == b).then_some(e.target)
                });
            let target = targets.next()?;
            if targets.any(|t| t != target) {
                return None;
            }
            let nested = invocation_paths(target, graph, inputs, root, active, all_exits)?;
            let mut next = Vec::new();
            for previous in &accumulated {
                for calls in &nested {
                    let mut combined = previous.clone();
                    combined.insert(target);
                    combined.extend(calls);
                    next.push(combined);
                }
            }
            if next.len() > 256 {
                return None;
            }
            accumulated = next;
        }
        result.extend(accumulated);
    }
    active.remove(&id);
    Some(result)
}
pub(crate) fn append(
    root: &Path,
    config: &Config,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    append_private_implementations(root, inputs, graph, findings, losses);
    let Some(policy) = &config.semantic_policy else {
        return;
    };
    for peer in &policy.peers {
        let all_exits = peer.exit_scope.as_deref() != Some("normal");
        let required: Option<Vec<_>> = peer
            .required_operations
            .iter()
            .map(|n| resolve(graph, root, n))
            .collect();
        for authority in &peer.subjects {
            let id = resolve(graph, root, authority);
            let subject = id.and_then(|id| graph.get_symbol(id));
            let input = subject.and_then(|s| inputs.iter().find(|i| i.path == s.location.file));
            let key = input.map_or_else(String::new, |i| path_key(root, &i.path));
            let range = subject
                .zip(input)
                .and_then(|(s, i)| Some(location_range(&key, i.content.as_deref()?, &s.location)));
            let paths =
                id.and_then(|id| invocation_paths(id, graph, inputs, root, &mut HashSet::new(), all_exits));
            let (Some(required), Some(paths)) = (&required, paths) else {
                let reason = if id.is_none() || required.is_none() {
                    "policy_binding_unresolved"
                } else {
                    "policy_control_unresolved"
                };
                let mut item = loss(reason, key, range, "semantic_policy");
                item["affected_patterns"] = json!(["partial_wiring"]);
                item["policy_subject"] = json!(authority);
                item["policy"] = json!(peer);
                item["id"] = json!(hash("policy.loss", &item));
                losses.push(item);
                continue;
            };
            let missing: Vec<_> = required
                .iter()
                .zip(&peer.required_operations)
                .filter(|(id, _)| paths.iter().any(|p| !p.contains(id)))
                .map(|(_, name)| name.clone())
                .collect();
            if missing.is_empty() {
                continue;
            }
            let subject = subject.unwrap();
            let input = input.unwrap();
            let content = input.content.as_deref().unwrap();
            let range = range.unwrap();
            let base = json!({"path_key":key,"language":input.language,"qualified_name":subject.qualified_name,"construct_kind":"function"});
            let cause = json!({"key_version":1,"parts":{"relation":if all_exits {"required-invocation-on-every-supported-exit-path"} else {"required-invocation-on-every-normal-return-path"},"policy_subjects":peer.subjects,"required_operations":peer.required_operations,"missing_operations":missing}});
            let semantic_id = hash(
                "finding.semantic",
                &json!({"pattern_id":"partial_wiring","subject":base,"cause":cause}),
            );
            let control = graph
                .semantic_files()
                .iter()
                .find(|f| f.path == input.path)
                .unwrap()
                .normal_control
                .iter()
                .find(|c| {
                    c.name == subject.name
                        && c.range.0 == range["start_byte"].as_u64().unwrap() as usize
                })
                .unwrap();
            let evidence = json!([{"kind":"declared_normal_exit_invocation","cfg_exit_scope":if all_exits {"all-supported-exits"} else {"normal"},"exceptional_paths":control.exceptional_paths,"policy":peer,"normal_paths":control.paths.iter().map(|p|p.iter().map(|&(a,b)|offset_range(&key,content,a as u64,b as u64)).collect::<Vec<_>>()).collect::<Vec<_>>(),"missing_operations":missing,"binding_targets":required.iter().filter_map(|id|graph.get_symbol(*id).map(|s|json!({"path_key":path_key(root,&s.location.file),"qualified_name":s.qualified_name}))).collect::<Vec<_>>()}]);
            let observation_id = hash(
                "finding.observation",
                &json!({"semantic_id":semantic_id,"input":digest(content.as_bytes()),"range":range,"evidence":evidence}),
            );
            let mut subject = base;
            subject["name"] = json!(graph.get_symbol(id.unwrap()).unwrap().name);
            findings.push(json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":"partial_wiring","severity":"warning","confidence":"high","subject":subject,"cause":cause,"range":range,"evidence":evidence,"suggestion":"Connect the declared operation on every normal-return peer path."}));
        }
    }
}

fn append_private_implementations(
    root: &Path,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    for (id, symbol) in graph
        .all_symbols()
        .filter(|(_, s)| s.kind == SymbolKind::Method)
    {
        let Some(input) = inputs
            .iter()
            .find(|i| i.path == symbol.location.file && i.parsed && !i.changed_during_run)
        else {
            continue;
        };
        let Some(content) = input.content.as_deref() else {
            continue;
        };
        let key = path_key(root, &input.path);
        let range = location_range(&key, content, &symbol.location);
        // An actual bound call/reference defeats absence, including an unrelated
        // spelling on a different owner. Type annotations alone are not consumers.
        if graph.edges_to(id).iter().any(|e| {
            matches!(e.kind, EdgeKind::Calls | EdgeKind::References)
                && e.reference_id
                    .and_then(|i| graph.get_reference(i))
                    .is_none_or(|r| r.kind != crate::parser::ir::ReferenceKind::Annotation)
        }) {
            continue;
        }
        let Some(proof) = graph.private_implementation(id) else {
            let mut item = loss(
                "implementation_dispatch_unresolved",
                key,
                Some(range),
                "semantic_prerequisite",
            );
            item["affected_patterns"] = json!(["data_dead_end", "orphaned_impl"]);
            item["required_capabilities"] = json!([
                "private owner escape/export and implementation dispatch/registration closure"
            ]);
            losses.push(item);
            continue;
        };
        let subject = json!({"path_key":key,"language":input.language,"qualified_name":symbol.qualified_name,"name":symbol.name,"construct_kind":"method"});
        let evidence = json!([{"kind":"private_nominal_implementation_closure","owner":proof.owner,"owner_escape":"none","owner_range":offset_range(&key,content,proof.owner_range.0 as u64,proof.owner_range.1 as u64),"implementation_range":offset_range(&key,content,proof.implementation_range.0 as u64,proof.implementation_range.1 as u64),"implementation_obligation":"private_inherent_implementation","dispatch_paths":[],"registration_paths":[],"selected_file_consumer_universe":"complete-single-file-original-bytes"}]);
        for pattern in ["data_dead_end", "orphaned_impl"] {
            let cause = json!({"key_version":1,"parts":{"owner":proof.owner,"relation":if pattern=="data_dead_end"{"unconsumed_private_executable"}else{"unregistered_private_implementation"}}});
            let semantic_id = hash(
                "finding.semantic",
                &json!({"pattern_id":pattern,"subject":subject,"cause":cause}),
            );
            let observation_id = hash(
                "finding.observation",
                &json!({"semantic_id":semantic_id,"source_sha256":digest(content.as_bytes()),"range":range,"evidence":evidence}),
            );
            findings.push(json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":pattern,"severity":"warning","confidence":"high","subject":subject,"cause":cause,"range":range,"evidence":evidence,"suggestion":"Connect the private implementation to its intended consumer or remove it."}));
        }
    }
}
