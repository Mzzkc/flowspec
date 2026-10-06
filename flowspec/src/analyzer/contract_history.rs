//! Binding-exact finite primitive contracts and supplied identity history.
use crate::report_authority::{
    digest, hash, location_range, loss, offset_range, path_key, InputRecord,
};
use crate::{
    config::Config,
    graph::Graph,
    parser::ir::{EdgeKind, SymbolId, SymbolKind},
};
use serde_json::{json, Value};
use std::path::Path;
fn input<'a>(inputs: &'a [InputRecord], path: &Path) -> Option<&'a InputRecord> {
    inputs
        .iter()
        .find(|i| i.path == path && i.parsed && !i.changed_during_run)
}
fn finding(
    pattern: &str,
    root: &Path,
    i: &InputRecord,
    name: &str,
    range: Value,
    cause: Value,
    evidence: Value,
) -> Value {
    let subject = json!({"path_key":path_key(root,&i.path),"language":i.language,"qualified_name":name,"name":name,"construct_kind":"bound_contract_use"});
    let semantic_id = hash(
        "finding.semantic",
        &json!({"pattern_id":pattern,"subject":subject,"cause":cause}),
    );
    let observation_id = hash(
        "finding.observation",
        &json!({"semantic_id":semantic_id,"source":digest(i.content.as_deref().unwrap().as_bytes()),"range":range,"evidence":evidence}),
    );
    json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":pattern,"severity":"warning","confidence":"high","subject":subject,"cause":{"key_version":1,"parts":cause},"range":range,"evidence":[evidence],"suggestion":"Reconcile the bound source use with its exact declared authority."})
}
fn debt(
    pattern: &str,
    reason: &str,
    root: &Path,
    i: Option<&InputRecord>,
    range: Option<Value>,
    policy: Value,
) -> Value {
    let mut l = loss(
        reason,
        i.map_or_else(String::new, |i| path_key(root, &i.path)),
        range,
        "semantic_prerequisite",
    );
    l["affected_patterns"] = json!([pattern]);
    l["policy"] = policy;
    l["id"] = json!(hash("contract-history.loss", &l));
    l
}
fn signature<'a>(
    graph: &'a Graph,
    root: &Path,
    inputs: &[InputRecord],
    id: SymbolId,
) -> Option<&'a crate::parser::contracts::SignatureFact> {
    let s = graph.get_symbol(id)?;
    let i = input(inputs, &s.location.file)?;
    let c = i.content.as_deref()?;
    let a =
        location_range(&path_key(root, &i.path), c, &s.location)["start_byte"].as_u64()? as usize;
    graph
        .semantic_files()
        .iter()
        .find(|f| f.path == i.path)?
        .signatures
        .iter()
        .find(|f| f.range.0 == a && f.name == s.name)
}
pub(crate) fn append(
    root: &Path,
    config: &Config,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    let contracts = config
        .semantic_policy
        .as_ref()
        .map(|p| p.contracts.as_slice())
        .unwrap_or(&[]);
    for rule in contracts {
        if super::semantic_policy::resolve(graph, root, &rule.target).is_none() {
            losses.push(debt(
                "contract_mismatch",
                "contract_binding_unresolved",
                root,
                None,
                None,
                json!(rule),
            ));
        }
    }
    for (id, s) in graph.all_symbols().filter(|(_, s)| {
        s.kind == SymbolKind::Function && !s.annotations.iter().any(|a| a == "import")
    }) {
        let Some(sig) = signature(graph, root, inputs, id) else {
            continue;
        };
        let Some(i) = input(inputs, &s.location.file) else {
            continue;
        };
        let content = i.content.as_deref().unwrap();
        let key = path_key(root, &i.path);
        let explicit = contracts
            .iter()
            .find(|r| super::semantic_policy::resolve(graph, root, &r.target) == Some(id));
        let python_builtin_shadow = i.language.as_deref() == Some("python")
            && graph.all_symbols().any(|(_, s)| {
                s.location.file == i.path && matches!(s.name.as_str(), "bool" | "str")
            });
        if python_builtin_shadow
            && (sig.return_type.is_some() || sig.parameter_types.iter().any(Option::is_some))
        {
            losses.push(debt(
                "contract_mismatch",
                "contract_type_binding_unresolved",
                root,
                Some(i),
                Some(offset_range(
                    &key,
                    content,
                    sig.range.0 as u64,
                    sig.range.1 as u64,
                )),
                json!({"native_signature":sig}),
            ));
        }
        let mut expected = if python_builtin_shadow {
            vec![None; sig.parameter_types.len()]
        } else {
            sig.parameter_types.clone()
        };
        if let Some(rule) = explicit {
            if sig.unsupported_parameters || rule.parameter_types.len() != sig.parameter_types.len()
            {
                losses.push(debt(
                    "contract_mismatch",
                    "contract_signature_unresolved",
                    root,
                    Some(i),
                    Some(offset_range(
                        &key,
                        content,
                        sig.range.0 as u64,
                        sig.range.1 as u64,
                    )),
                    json!(rule),
                ));
                continue;
            }
            for (index, ty) in rule.parameter_types.iter().enumerate() {
                if !python_builtin_shadow
                    && sig.parameter_types[index]
                        .as_ref()
                        .is_some_and(|native| native != ty)
                {
                    findings.push(finding("contract_mismatch",root,i,&s.qualified_name,offset_range(&key,content,sig.range.0 as u64,sig.range.1 as u64),json!({"relation":"native-signature-schema-contradiction","parameter":index,"native":sig.parameter_types[index],"schema":ty}),json!({"kind":"native_signature_and_declared_schema","signature":sig,"schema":rule,"runtime_claim":false})));
                }
                expected[index] = Some(ty.clone());
            }
        }
        if let Some(return_type) = sig.return_type.as_ref().filter(|_| !python_builtin_shadow) {
            for (r, actual) in &sig.literal_returns {
                if actual != return_type {
                    findings.push(finding("contract_mismatch",root,i,&s.qualified_name,offset_range(&key,content,r.0 as u64,r.1 as u64),json!({"relation":"literal-return-type-contradiction","expected":return_type,"actual":actual}),json!({"kind":"native_signature_literal_return","declaration_range":offset_range(&key,content,sig.range.0 as u64,sig.range.1 as u64),"declared_type":return_type,"literal_type":actual,"runtime_claim":false})));
                }
            }
        }
        if expected.iter().all(Option::is_none) {
            continue;
        }
        for edge in graph
            .consumption_edges_to(id)
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls)
        {
            let Some(r) = edge.reference_id.and_then(|r| graph.get_reference(r)) else {
                continue;
            };
            let Some(ci) = input(inputs, &r.location.file) else {
                continue;
            };
            let cc = ci.content.as_deref().unwrap();
            let ck = path_key(root, &ci.path);
            let range = location_range(&ck, cc, &r.location);
            let literal = graph
                .semantic_files()
                .iter()
                .find(|f| f.path == ci.path)
                .and_then(|f| {
                    f.literal_calls.iter().find(|c| {
                        range["start_byte"] == c.range.0 && range["end_byte"] == c.range.1
                    })
                });
            let Some(call) = literal else {
                losses.push(debt(
                    "contract_mismatch",
                    "contract_operand_unresolved",
                    root,
                    Some(ci),
                    Some(range),
                    json!({"target":s.qualified_name}),
                ));
                continue;
            };
            if sig.unsupported_parameters || call.arguments.len() != expected.len() {
                losses.push(debt(
                    "contract_mismatch",
                    "contract_signature_unresolved",
                    root,
                    Some(ci),
                    Some(range),
                    json!({"target":s.qualified_name}),
                ));
                continue;
            }
            for (n, (want, actual)) in expected.iter().zip(&call.arguments).enumerate() {
                let Some(want) = want else { continue };
                let Some(actual) = actual else {
                    losses.push(debt(
                        "contract_mismatch",
                        "contract_operand_unresolved",
                        root,
                        Some(ci),
                        Some(range.clone()),
                        json!({"target":s.qualified_name,"parameter":n}),
                    ));
                    continue;
                };
                if want != actual {
                    findings.push(finding("contract_mismatch",root,ci,&s.qualified_name,range.clone(),json!({"relation":"bound-literal-argument-type-contradiction","target_path":key,"target":s.qualified_name,"parameter":n,"expected":want,"actual":actual}),json!({"kind":"native_bound_literal_operand","binding_target":{"path_key":key,"qualified_name":s.qualified_name,"declaration_range":offset_range(&key,content,sig.range.0 as u64,sig.range.1 as u64)},"native_signature":sig,"declared_schema":explicit,"operand_types":call.arguments,"runtime_claim":false})));
                }
            }
        }
    }
    let Some(policy) = &config.semantic_policy else {
        return;
    };
    for h in &policy.history {
        let old = super::semantic_policy::resolve(graph, root, &h.old);
        let new = h
            .new
            .as_deref()
            .and_then(|n| super::semantic_policy::resolve(graph, root, n));
        let Some(old) = old else {
            losses.push(debt(
                "stale_reference",
                "history_old_binding_unresolved",
                root,
                None,
                None,
                json!(h),
            ));
            continue;
        };
        let s = graph.get_symbol(old).unwrap();
        let Some(i) = input(inputs, &s.location.file) else {
            continue;
        };
        if h.state != "shim"
            || new.is_none()
            || !graph
                .call_edges_from(old)
                .iter()
                .any(|e| e.kind == EdgeKind::Calls && Some(e.target) == new)
        {
            losses.push(debt(
                "stale_reference",
                "history_transition_unresolved",
                root,
                Some(i),
                Some(location_range(
                    &path_key(root, &i.path),
                    i.content.as_deref().unwrap(),
                    &s.location,
                )),
                json!(h),
            ));
            continue;
        }
        for e in graph
            .consumption_edges_to(old)
            .iter()
            .filter(|e| e.kind == EdgeKind::Calls && Some(e.target) != new && e.target != old)
        {
            let Some(r) = e.reference_id.and_then(|id| graph.get_reference(id)) else {
                continue;
            };
            let Some(ci) = input(inputs, &r.location.file) else {
                continue;
            };
            let range = location_range(
                &path_key(root, &ci.path),
                ci.content.as_deref().unwrap(),
                &r.location,
            );
            findings.push(finding("stale_reference",root,ci,&s.qualified_name,range,json!({"relation":"live-call-to-supplied-historical-shim","history":h}),json!({"kind":"supplied_identity_history_bound_consumer","history":h,"old_binding":{"path_key":path_key(root,&s.location.file),"qualified_name":s.qualified_name},"replacement_binding":new.and_then(|id|graph.get_symbol(id)).map(|s|json!({"path_key":path_key(root,&s.location.file),"qualified_name":s.qualified_name})),"history_authority":"validated-user-supplied-event","consumer_kind":"exact-bound-call"})));
        }
    }
}
