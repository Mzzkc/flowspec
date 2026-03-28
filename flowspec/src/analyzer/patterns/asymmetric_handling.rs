// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Asymmetric handling detection — parallel code paths with inconsistent treatment.
//!
//! Detects groups of sibling functions (same file, same kind, similar arity)
//! where most members call a shared set of functions but one or more members
//! are missing consensus callees. The classic case: 3 of 4 API handlers call
//! `validate()` but the 4th doesn't.
//!
//! Grouping is restricted to same-file functions to avoid cross-module false
//! positives. Minimum group size is 3 — pairs are too ambiguous. Confidence
//! is always `Moderate` because "parallel" is a structural judgment.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, is_test_path, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{SymbolId, SymbolKind};

/// Minimum number of siblings required to form a group for comparison.
const MIN_GROUP_SIZE: usize = 3;

/// Maximum arity difference for two functions to be considered "similar arity".
const MAX_ARITY_DIFF: usize = 1;

/// Detect asymmetric handling in the analysis graph.
///
/// Groups functions by file, kind, and similar arity. For each group of ≥3
/// siblings, computes "consensus callees" — functions called by a supermajority
/// (≥ N-1 of N members). Reports any member missing a consensus callee.
///
/// Restricted to same-file grouping to avoid cross-module false positives.
/// Confidence is always `Moderate` — structural parallelism is a heuristic.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Group candidate functions by (file, kind)
    type FunctionGroup = Vec<(SymbolId, String, usize)>;
    let mut file_kind_groups: HashMap<(String, SymbolKind), FunctionGroup> = HashMap::new();

    for (id, symbol) in graph.all_symbols() {
        if is_excluded_symbol(symbol) {
            continue;
        }

        if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
            continue;
        }

        let file_str = symbol.location.file.to_string_lossy().to_string();
        if is_test_path(&file_str) {
            continue;
        }

        let arity = extract_arity(symbol.signature.as_deref());
        let key = (file_str, symbol.kind);
        file_kind_groups
            .entry(key)
            .or_default()
            .push((id, symbol.qualified_name.clone(), arity));
    }

    // For each (file, kind) group, sub-group by similar arity
    for ((_file, _kind), members) in &file_kind_groups {
        // Build arity-compatible sub-groups
        let sub_groups = build_arity_groups(members);

        for group in sub_groups {
            if group.len() < MIN_GROUP_SIZE {
                continue;
            }

            // Compute callees for each member
            let member_callees: Vec<(SymbolId, HashSet<SymbolId>)> = group
                .iter()
                .map(|&(id, _, _)| {
                    let callees: HashSet<SymbolId> = graph.callees(id).into_iter().collect();
                    (id, callees)
                })
                .collect();

            // Compute consensus callees: present in ≥ (N-1) of N members
            let n = member_callees.len();
            let threshold = n - 1; // supermajority

            let mut callee_counts: HashMap<SymbolId, usize> = HashMap::new();
            for (_, callees) in &member_callees {
                for &callee_id in callees {
                    *callee_counts.entry(callee_id).or_insert(0) += 1;
                }
            }

            let consensus: HashSet<SymbolId> = callee_counts
                .iter()
                .filter(|&(_, &count)| count >= threshold)
                .map(|(&id, _)| id)
                .collect();

            if consensus.is_empty() {
                continue;
            }

            // Check each member for missing consensus callees
            for (member_id, member_callees_set) in &member_callees {
                let missing: Vec<SymbolId> = consensus
                    .iter()
                    .filter(|&&c| !member_callees_set.contains(&c))
                    .copied()
                    .collect();

                if missing.is_empty() {
                    continue;
                }

                let member_sym = match graph.get_symbol(*member_id) {
                    Some(s) => s,
                    None => continue,
                };

                let missing_names: Vec<String> = missing
                    .iter()
                    .filter_map(|&mid| graph.get_symbol(mid).map(|s| s.name.clone()))
                    .collect();

                let sibling_names: Vec<String> = group
                    .iter()
                    .filter(|(id, _, _)| *id != *member_id)
                    .map(|(_, name, _)| name.clone())
                    .collect();

                let location = format!(
                    "{}:{}",
                    relativize_path(&member_sym.location.file, project_root),
                    member_sym.location.line
                );

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::AsymmetricHandling,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: member_sym.qualified_name.clone(),
                    message: format!(
                        "Asymmetric handling: '{}' is missing [{}] which {}/{} sibling(s) call",
                        member_sym.name,
                        missing_names.join(", "),
                        n - 1,
                        n
                    ),
                    evidence: vec![
                        Evidence {
                            observation: format!(
                                "Missing consensus callee(s): [{}]. Present in {}/{} siblings.",
                                missing_names.join(", "),
                                threshold,
                                n
                            ),
                            location: Some(location.clone()),
                            context: Some(format!(
                                "Siblings: [{}]",
                                sibling_names.join(", ")
                            )),
                        },
                    ],
                    suggestion: format!(
                        "Verify that '{}' intentionally omits [{}] — all {} sibling function(s) call it",
                        member_sym.name,
                        missing_names.join(", "),
                        threshold
                    ),
                    location,
                });
            }
        }
    }

    diagnostics
}

/// Build sub-groups of functions with similar arity (within MAX_ARITY_DIFF).
///
/// Uses a greedy approach: sort by arity, then group consecutive entries
/// that are within the arity tolerance of the group's anchor.
fn build_arity_groups(
    members: &[(SymbolId, String, usize)],
) -> Vec<Vec<(SymbolId, String, usize)>> {
    if members.is_empty() {
        return Vec::new();
    }

    let mut sorted: Vec<(SymbolId, String, usize)> = members.to_vec();
    sorted.sort_by_key(|&(_, _, arity)| arity);

    let mut groups: Vec<Vec<(SymbolId, String, usize)>> = Vec::new();
    let mut current_group: Vec<(SymbolId, String, usize)> = vec![sorted[0].clone()];
    let mut anchor_arity = sorted[0].2;

    for entry in sorted.iter().skip(1) {
        if entry.2.abs_diff(anchor_arity) <= MAX_ARITY_DIFF {
            current_group.push(entry.clone());
        } else {
            if current_group.len() >= MIN_GROUP_SIZE {
                groups.push(std::mem::take(&mut current_group));
            } else {
                current_group.clear();
            }
            current_group.push(entry.clone());
            anchor_arity = entry.2;
        }
    }

    if current_group.len() >= MIN_GROUP_SIZE {
        groups.push(current_group);
    }

    groups
}

/// Extract parameter arity from a function signature string.
fn extract_arity(signature: Option<&str>) -> usize {
    let Some(sig) = signature else {
        return 0;
    };
    let sig = sig.trim();
    if !sig.starts_with('(') {
        return 0;
    }

    let inner = if let Some(close) = sig.find(')') {
        &sig[1..close]
    } else {
        return 0;
    };

    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return 0;
    }

    trimmed.split(',').filter(|p| !p.trim().is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ir::{ReferenceKind, Visibility};
    use crate::test_utils::*;
    use std::path::PathBuf;

    // =========================================================================
    // 2. Asymmetric Handling Pattern Tests
    // =========================================================================

    // -- 2.1 True Positive: One sibling missing consensus callee --

    #[test]
    fn test_asymmetric_true_positive_missing_validate() {
        let mut g = Graph::new();
        let f = "api/handlers.py";

        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let authorize = g.add_symbol(make_symbol(
            "authorize",
            SymbolKind::Function,
            Visibility::Private,
            f,
            10,
        ));
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));
        let update = g.add_symbol(make_symbol(
            "update",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));
        let delete = g.add_symbol(make_symbol(
            "delete",
            SymbolKind::Function,
            Visibility::Private,
            f,
            25,
        ));
        let fetch = g.add_symbol(make_symbol(
            "fetch",
            SymbolKind::Function,
            Visibility::Private,
            f,
            30,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            f,
            35,
        ));

        let mut hc = make_symbol(
            "handle_create",
            SymbolKind::Function,
            Visibility::Public,
            f,
            40,
        );
        hc.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hc_id = g.add_symbol(hc);

        let mut hu = make_symbol(
            "handle_update",
            SymbolKind::Function,
            Visibility::Public,
            f,
            50,
        );
        hu.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hu_id = g.add_symbol(hu);

        let mut hd = make_symbol(
            "handle_delete",
            SymbolKind::Function,
            Visibility::Public,
            f,
            60,
        );
        hd.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hd_id = g.add_symbol(hd);

        let mut hr = make_symbol(
            "handle_read",
            SymbolKind::Function,
            Visibility::Public,
            f,
            70,
        );
        hr.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hr_id = g.add_symbol(hr);

        // Wire calls
        add_ref(&mut g, hc_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, hc_id, authorize, ReferenceKind::Call, f);
        add_ref(&mut g, hc_id, save, ReferenceKind::Call, f);
        add_ref(&mut g, hc_id, respond, ReferenceKind::Call, f);

        add_ref(&mut g, hu_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, hu_id, authorize, ReferenceKind::Call, f);
        add_ref(&mut g, hu_id, update, ReferenceKind::Call, f);
        add_ref(&mut g, hu_id, respond, ReferenceKind::Call, f);

        // handle_delete — MISSING validate
        add_ref(&mut g, hd_id, authorize, ReferenceKind::Call, f);
        add_ref(&mut g, hd_id, delete, ReferenceKind::Call, f);
        add_ref(&mut g, hd_id, respond, ReferenceKind::Call, f);

        add_ref(&mut g, hr_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, hr_id, authorize, ReferenceKind::Call, f);
        add_ref(&mut g, hr_id, fetch, ReferenceKind::Call, f);
        add_ref(&mut g, hr_id, respond, ReferenceKind::Call, f);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);

        assert!(
            results.iter().any(|d| {
                d.pattern == DiagnosticPattern::AsymmetricHandling
                    && d.entity.contains("handle_delete")
            }),
            "Expected asymmetric handling finding for handle_delete missing validate, got: {:?}",
            results
        );

        let diag = results
            .iter()
            .find(|d| {
                d.pattern == DiagnosticPattern::AsymmetricHandling
                    && d.entity.contains("handle_delete")
            })
            .unwrap();
        assert_eq!(diag.severity, Severity::Warning);
        assert_eq!(diag.confidence, Confidence::Moderate);
        assert!(!diag.evidence.is_empty());
    }

    // -- 2.2 True Negative: Intentionally different functions --

    #[test]
    fn test_asymmetric_true_negative_different_roles() {
        let mut g = Graph::new();
        let f = "app.py";

        let connect = g.add_symbol(make_symbol(
            "connect",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        let migrate = g.add_symbol(make_symbol(
            "migrate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            2,
        ));
        let seed = g.add_symbol(make_symbol(
            "seed",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));
        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            4,
        ));
        let process = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            f,
            6,
        ));
        let close = g.add_symbol(make_symbol(
            "close",
            SymbolKind::Function,
            Visibility::Private,
            f,
            7,
        ));
        let flush = g.add_symbol(make_symbol(
            "flush",
            SymbolKind::Function,
            Visibility::Private,
            f,
            8,
        ));
        let deallocate = g.add_symbol(make_symbol(
            "deallocate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            9,
        ));

        let mut setup_db = make_symbol("setup_db", SymbolKind::Function, Visibility::Public, f, 10);
        setup_db.signature = Some("(config) -> None".to_string());
        let sd_id = g.add_symbol(setup_db);

        let mut handle_request = make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        );
        handle_request.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hr_id = g.add_symbol(handle_request);

        let mut cleanup = make_symbol("cleanup", SymbolKind::Function, Visibility::Public, f, 30);
        cleanup.signature = Some("(resources) -> None".to_string());
        let cl_id = g.add_symbol(cleanup);

        add_ref(&mut g, sd_id, connect, ReferenceKind::Call, f);
        add_ref(&mut g, sd_id, migrate, ReferenceKind::Call, f);
        add_ref(&mut g, sd_id, seed, ReferenceKind::Call, f);

        add_ref(&mut g, hr_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, hr_id, process, ReferenceKind::Call, f);
        add_ref(&mut g, hr_id, respond, ReferenceKind::Call, f);

        add_ref(&mut g, cl_id, close, ReferenceKind::Call, f);
        add_ref(&mut g, cl_id, flush, ReferenceKind::Call, f);
        add_ref(&mut g, cl_id, deallocate, ReferenceKind::Call, f);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "Different-role functions should NOT trigger asymmetric handling"
        );
    }

    // -- 2.3 Adversarial: Group of 2 (below minimum) --

    #[test]
    fn test_asymmetric_adversarial_group_too_small() {
        let mut g = Graph::new();
        let f = "handlers.py";

        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        let transform = g.add_symbol(make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Private,
            f,
            2,
        ));
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut process_a =
            make_symbol("process_a", SymbolKind::Function, Visibility::Public, f, 10);
        process_a.signature = Some("(data: dict, config: Config) -> Result".to_string());
        let pa_id = g.add_symbol(process_a);

        let mut process_b =
            make_symbol("process_b", SymbolKind::Function, Visibility::Public, f, 20);
        process_b.signature = Some("(data: dict, config: Config) -> Result".to_string());
        let pb_id = g.add_symbol(process_b);

        add_ref(&mut g, pa_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, pa_id, transform, ReferenceKind::Call, f);
        add_ref(&mut g, pa_id, save, ReferenceKind::Call, f);

        add_ref(&mut g, pb_id, transform, ReferenceKind::Call, f);
        add_ref(&mut g, pb_id, save, ReferenceKind::Call, f);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "Group of 2 should NOT trigger asymmetric handling (min group size 3)"
        );
    }

    // -- 2.4 Adversarial: Cross-module grouping should not happen --

    #[test]
    fn test_asymmetric_adversarial_cross_module_no_grouping() {
        let mut g = Graph::new();

        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            "auth/handlers.py",
            1,
        ));
        let authenticate = g.add_symbol(make_symbol(
            "authenticate",
            SymbolKind::Function,
            Visibility::Private,
            "auth/handlers.py",
            2,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            "auth/handlers.py",
            3,
        ));
        let create = g.add_symbol(make_symbol(
            "create",
            SymbolKind::Function,
            Visibility::Private,
            "user/handlers.py",
            1,
        ));
        let respond2 = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            "user/handlers.py",
            2,
        ));
        let validate2 = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            "user/handlers.py",
            3,
        ));
        let create2 = g.add_symbol(make_symbol(
            "create",
            SymbolKind::Function,
            Visibility::Private,
            "payment/handlers.py",
            1,
        ));
        let respond3 = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            "payment/handlers.py",
            2,
        ));

        let mut h1 = make_symbol(
            "handle_login",
            SymbolKind::Function,
            Visibility::Public,
            "auth/handlers.py",
            10,
        );
        h1.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let h1_id = g.add_symbol(h1);

        let mut h2 = make_symbol(
            "handle_register",
            SymbolKind::Function,
            Visibility::Public,
            "user/handlers.py",
            10,
        );
        h2.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let h2_id = g.add_symbol(h2);

        let mut h3 = make_symbol(
            "handle_charge",
            SymbolKind::Function,
            Visibility::Public,
            "payment/handlers.py",
            10,
        );
        h3.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let h3_id = g.add_symbol(h3);

        add_ref(
            &mut g,
            h1_id,
            validate,
            ReferenceKind::Call,
            "auth/handlers.py",
        );
        add_ref(
            &mut g,
            h1_id,
            authenticate,
            ReferenceKind::Call,
            "auth/handlers.py",
        );
        add_ref(
            &mut g,
            h1_id,
            respond,
            ReferenceKind::Call,
            "auth/handlers.py",
        );

        add_ref(
            &mut g,
            h2_id,
            validate2,
            ReferenceKind::Call,
            "user/handlers.py",
        );
        add_ref(
            &mut g,
            h2_id,
            create,
            ReferenceKind::Call,
            "user/handlers.py",
        );
        add_ref(
            &mut g,
            h2_id,
            respond2,
            ReferenceKind::Call,
            "user/handlers.py",
        );

        add_ref(
            &mut g,
            h3_id,
            create2,
            ReferenceKind::Call,
            "payment/handlers.py",
        );
        add_ref(
            &mut g,
            h3_id,
            respond3,
            ReferenceKind::Call,
            "payment/handlers.py",
        );

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "Cross-module functions should NOT be grouped (same-file restriction)"
        );
    }

    // -- 2.5 Adversarial: All siblings identical (no asymmetry) --

    #[test]
    fn test_asymmetric_adversarial_all_identical_no_finding() {
        let mut g = Graph::new();
        let f = "handlers.py";

        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        let process = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Private,
            f,
            2,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let handlers: Vec<&str> = vec!["handler_a", "handler_b", "handler_c", "handler_d"];
        for (i, name) in handlers.iter().enumerate() {
            let mut h = make_symbol(
                name,
                SymbolKind::Function,
                Visibility::Public,
                f,
                (10 + i * 10) as u32,
            );
            h.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
            let h_id = g.add_symbol(h);
            add_ref(&mut g, h_id, validate, ReferenceKind::Call, f);
            add_ref(&mut g, h_id, process, ReferenceKind::Call, f);
            add_ref(&mut g, h_id, respond, ReferenceKind::Call, f);
        }

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "All-identical siblings should NOT trigger asymmetric handling"
        );
    }

    // -- 2.6 Adversarial: Mixed kind should not group --

    #[test]
    fn test_asymmetric_adversarial_mixed_kind_no_grouping() {
        let mut g = Graph::new();
        let f = "service.py";

        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            2,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut process_fn =
            make_symbol("process", SymbolKind::Function, Visibility::Public, f, 10);
        process_fn.signature = Some("(data: dict, ctx: Context) -> Result".to_string());
        let pf_id = g.add_symbol(process_fn);

        let mut handle_fn = make_symbol("handle", SymbolKind::Function, Visibility::Public, f, 20);
        handle_fn.signature = Some("(data: dict, ctx: Context) -> Result".to_string());
        let hf_id = g.add_symbol(handle_fn);

        // run is a Method, not a Function
        let mut run_method = make_symbol("run", SymbolKind::Method, Visibility::Public, f, 30);
        run_method.signature = Some("(data: dict, ctx: Context) -> Result".to_string());
        let rm_id = g.add_symbol(run_method);

        add_ref(&mut g, pf_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, pf_id, save, ReferenceKind::Call, f);

        add_ref(&mut g, hf_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, hf_id, respond, ReferenceKind::Call, f);

        add_ref(&mut g, rm_id, respond, ReferenceKind::Call, f);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::AsymmetricHandling),
            "Mixed kinds (Function + Method) with only 2 of each should NOT form a valid group"
        );
    }

    // -- 2.7 True Positive: Consensus threshold with 5 siblings --

    #[test]
    fn test_asymmetric_true_positive_two_missing_callees() {
        let mut g = Graph::new();
        let f = "api/handlers.py";

        let auth = g.add_symbol(make_symbol(
            "auth",
            SymbolKind::Function,
            Visibility::Private,
            f,
            1,
        ));
        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            2,
        ));
        let log = g.add_symbol(make_symbol(
            "log",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));
        let process = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Private,
            f,
            4,
        ));
        let respond = g.add_symbol(make_symbol(
            "respond",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let transform = g.add_symbol(make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Private,
            f,
            6,
        ));
        let fetch = g.add_symbol(make_symbol(
            "fetch",
            SymbolKind::Function,
            Visibility::Private,
            f,
            7,
        ));
        let delete_fn = g.add_symbol(make_symbol(
            "delete_fn",
            SymbolKind::Function,
            Visibility::Private,
            f,
            8,
        ));
        let create = g.add_symbol(make_symbol(
            "create",
            SymbolKind::Function,
            Visibility::Private,
            f,
            9,
        ));

        let names = ["h1", "h2", "h3", "h4", "h5"];
        let mut ids = Vec::new();
        for (i, name) in names.iter().enumerate() {
            let mut h = make_symbol(
                name,
                SymbolKind::Function,
                Visibility::Public,
                f,
                (40 + i * 10) as u32,
            );
            h.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
            ids.push(g.add_symbol(h));
        }

        // h1: auth, validate, log, process, respond
        for callee in [auth, validate, log, process, respond] {
            add_ref(&mut g, ids[0], callee, ReferenceKind::Call, f);
        }
        // h2: auth, validate, log, transform, respond
        for callee in [auth, validate, log, transform, respond] {
            add_ref(&mut g, ids[1], callee, ReferenceKind::Call, f);
        }
        // h3: auth, validate, log, fetch, respond
        for callee in [auth, validate, log, fetch, respond] {
            add_ref(&mut g, ids[2], callee, ReferenceKind::Call, f);
        }
        // h4: auth, log, delete_fn, respond — MISSING validate
        for callee in [auth, log, delete_fn, respond] {
            add_ref(&mut g, ids[3], callee, ReferenceKind::Call, f);
        }
        // h5: auth, validate, log, create, respond
        for callee in [auth, validate, log, create, respond] {
            add_ref(&mut g, ids[4], callee, ReferenceKind::Call, f);
        }

        // Consensus (≥4 of 5): auth, validate, log, respond
        // h4 is missing validate

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            results.iter().any(|d| {
                d.pattern == DiagnosticPattern::AsymmetricHandling && d.entity.contains("h4")
            }),
            "h4 missing 'validate' (consensus callee) should trigger, got: {:?}",
            results
        );
    }
}
