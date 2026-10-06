//! Finite linear resource obligations over exact ordered native call paths.
use crate::report_authority::{
    digest, hash, location_range, loss, offset_range, path_key, InputRecord,
};
use crate::{
    config::Config,
    graph::Graph,
    parser::ir::{EdgeKind, SymbolId},
};
use serde_json::{json, Value};
use std::path::Path;
fn paths(
    id: SymbolId,
    root: &Path,
    inputs: &[InputRecord],
    graph: &Graph,
) -> Option<Vec<Vec<SymbolId>>> {
    let s = graph.get_symbol(id)?;
    let i = inputs
        .iter()
        .find(|i| i.path == s.location.file && i.parsed && !i.changed_during_run)?;
    let content = i.content.as_deref()?;
    let key = path_key(root, &i.path);
    let start = location_range(&key, content, &s.location)["start_byte"].as_u64()? as usize;
    let f = graph.semantic_files().iter().find(|f| f.path == i.path)?;
    let c = f
        .normal_control
        .iter()
        .find(|c| c.name == s.name && c.range.0 == start)?;
    if !c.unresolved.is_empty() {
        return None;
    }
    c.all_paths
        .iter()
        .map(|p| {
            p.iter()
                .map(|&(a, b)| {
                    let mut targets = graph
                        .call_edges_from(id)
                        .into_iter()
                        .filter(|e| e.kind == EdgeKind::Calls)
                        .filter_map(|e| {
                            let r = graph.get_reference(e.reference_id?)?;
                            let rr = location_range(&key, content, &r.location);
                            (rr["start_byte"] == a && rr["end_byte"] == b).then_some(e.target)
                        });
                    let target = targets.next()?;
                    if targets.any(|t| t != target) {
                        return None;
                    }
                    // Endpoint/irrelevant calls are only literal, zero-argument leaf functions.
                    // This proves they cannot throw, diverge or hide further protocol effects.
                    let s = graph.get_symbol(target)?;
                    let fi = graph
                        .semantic_files()
                        .iter()
                        .find(|f| f.path == s.location.file)?;
                    let ii = inputs
                        .iter()
                        .find(|i| i.path == fi.path && i.parsed && !i.changed_during_run)?;
                    let ss = location_range(
                        &path_key(root, &ii.path),
                        ii.content.as_deref()?,
                        &s.location,
                    )["start_byte"]
                        .as_u64()? as usize;
                    let cc = fi
                        .normal_control
                        .iter()
                        .find(|c| c.range.0 == ss && c.name == s.name)?;
                    let sig = fi
                        .signatures
                        .iter()
                        .find(|c| c.range.0 == ss && c.name == s.name)?;
                    if !cc.unresolved.is_empty()
                        || cc.exceptional_paths > 0
                        || cc.all_paths.iter().any(|p| !p.is_empty())
                        || sig.unsupported_parameters
                        || !sig.parameter_types.is_empty()
                        || !sig.literal_leaf
                    {
                        return None;
                    }
                    Some(target)
                })
                .collect()
        })
        .collect()
}
pub(crate) fn append(
    root: &Path,
    config: &Config,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    let Some(policy) = &config.semantic_policy else {
        return;
    };
    for rule in &policy.resources {
        let acquire = super::semantic_policy::resolve(graph, root, &rule.acquire);
        let release = super::semantic_policy::resolve(graph, root, &rule.release);
        let mut checked = Vec::new();
        let mut uncertain = false;
        for authority in &rule.subjects {
            let id = super::semantic_policy::resolve(graph, root, authority);
            let s = id.and_then(|id| graph.get_symbol(id));
            let i = s.and_then(|s| inputs.iter().find(|i| i.path == s.location.file));
            let range = s.zip(i).and_then(|(s, i)| {
                Some(location_range(
                    &path_key(root, &i.path),
                    i.content.as_deref()?,
                    &s.location,
                ))
            });
            let p = id.and_then(|id| paths(id, root, inputs, graph));
            let (Some(a), Some(r), Some(p)) = (acquire, release, p) else {
                uncertain = true;
                let mut l = loss(
                    if id.is_none() || acquire.is_none() || release.is_none() {
                        "resource_binding_unresolved"
                    } else {
                        "resource_control_unresolved"
                    },
                    i.map_or_else(String::new, |i| path_key(root, &i.path)),
                    range,
                    "semantic_prerequisite",
                );
                l["affected_patterns"] = json!(["asymmetric_handling"]);
                l["policy"] = json!(rule);
                l["policy_subject"] = json!(authority);
                l["id"] = json!(hash("resource.loss", &l));
                losses.push(l);
                continue;
            };
            let mut bad = Vec::new();
            for (index, p) in p.iter().enumerate() {
                let mut balance = 0i64;
                let mut underflow = false;
                for op in p {
                    if *op == a {
                        balance += 1;
                    }
                    if *op == r {
                        if balance == 0 {
                            underflow = true;
                        } else {
                            balance -= 1;
                        }
                    }
                }
                if balance != 0 || underflow {
                    bad.push(json!({"path_index":index,"outstanding_acquisitions":balance,"release_without_acquisition":underflow}));
                }
            }
            checked.push((s.unwrap(), i.unwrap(), range.unwrap(), p, bad));
        }
        // Asymmetry needs at least one completely satisfied peer, and all peer
        // prerequisites closed. Equal failure is a held invariant, not asymmetry.
        if uncertain {
            continue;
        }
        if !checked.iter().any(|(_, _, _, _, bad)| bad.is_empty()) {
            let mut l = loss(
                "resource_no_satisfied_peer",
                String::new(),
                None,
                "semantic_prerequisite",
            );
            l["affected_patterns"] = json!(["asymmetric_handling"]);
            l["policy"] = json!(rule);
            l["id"] = json!(hash("resource.loss", &l));
            losses.push(l);
            continue;
        }
        for (s, i, range, p, bad) in checked {
            if bad.is_empty() {
                continue;
            }
            let key = path_key(root, &i.path);
            let content = i.content.as_deref().unwrap();
            let control = graph
                .semantic_files()
                .iter()
                .find(|f| f.path == i.path)
                .unwrap()
                .normal_control
                .iter()
                .find(|c| {
                    c.range.0 == range["start_byte"].as_u64().unwrap() as usize && c.name == s.name
                })
                .unwrap();
            let subject = json!({"path_key":key,"language":i.language,"qualified_name":s.qualified_name,"name":s.name,"construct_kind":"function"});
            let cause = json!({"key_version":1,"parts":{"relation":"declared-linear-resource-peer-invariant","policy":rule,"violations":bad}});
            let semantic_id = hash(
                "finding.semantic",
                &json!({"pattern_id":"asymmetric_handling","subject":subject,"cause":cause}),
            );
            let evidence = json!([{"kind":"bound_ordered_resource_all_exit_paths","policy":rule,"cfg_exit_scope":"all-supported-exits","exceptional_paths":control.exceptional_paths,"paths":p.iter().map(|p|p.iter().filter_map(|id|graph.get_symbol(*id).map(|s|json!({"path_key":path_key(root,&s.location.file),"qualified_name":s.qualified_name}))).collect::<Vec<_>>()).collect::<Vec<_>>(),"call_ranges":control.all_paths.iter().map(|p|p.iter().map(|&(a,b)|offset_range(&key,content,a as u64,b as u64)).collect::<Vec<_>>()).collect::<Vec<_>>(),"violations":bad,"endpoint_protocol":"declared-abstract-acquire-release; literal-zero-argument-no-throw-leaves"}]);
            let observation_id = hash(
                "finding.observation",
                &json!({"semantic_id":semantic_id,"source":digest(content.as_bytes()),"range":range,"evidence":evidence}),
            );
            findings.push(json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":"asymmetric_handling","severity":"warning","confidence":"high","subject":subject,"cause":cause,"range":range,"evidence":evidence,"suggestion":"Release each declared acquisition on every supported exit path."}));
        }
    }
}
