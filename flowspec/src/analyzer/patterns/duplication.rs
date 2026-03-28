// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Duplication detection — structural similarity in the IR.
//!
//! Detects functions with similar call patterns and matching arity.
//! Uses Jaccard similarity on callees sets (NOT textual similarity).
//! Self-recursive calls are excluded from the callees set to avoid
//! inflating the set for recursive functions.
//!
//! Confidence is always `Moderate` — structural similarity doesn't
//! always mean duplication. The evidence includes the Jaccard coefficient
//! and specific shared callees so consumers can assess each finding.

use std::collections::HashSet;
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, is_test_path, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{SymbolId, SymbolKind};

/// Minimum Jaccard similarity threshold for reporting duplication.
const JACCARD_THRESHOLD: f64 = 0.7;

/// Minimum number of callees in the union set for a meaningful comparison.
/// Below this, shared callees are likely coincidental API usage.
const MIN_CALLEES_FOR_COMPARISON: usize = 3;

/// Detect structural duplication between functions in the analysis graph.
///
/// Two functions are considered structurally duplicated when:
/// 1. They are in the same file (same-module comparison)
/// 2. They have matching arity (same parameter count)
/// 3. Their callees sets have a Jaccard similarity ≥ 0.7
///
/// Self-recursive calls are excluded from callees sets. Functions with
/// zero callees are skipped (no structural evidence). Test files are excluded.
///
/// Confidence is always `Moderate` per spec — structural similarity is a
/// heuristic, not a proof of actual code duplication.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    // Group candidate functions by file
    let mut file_groups: std::collections::HashMap<
        String,
        Vec<(SymbolId, &crate::parser::ir::Symbol)>,
    > = std::collections::HashMap::new();

    for (id, symbol) in graph.all_symbols() {
        // Skip excluded symbols
        if is_excluded_symbol(symbol) {
            continue;
        }

        // Only consider functions and methods
        if !matches!(symbol.kind, SymbolKind::Function | SymbolKind::Method) {
            continue;
        }

        let file_str = symbol.location.file.to_string_lossy().to_string();

        // Skip test files
        if is_test_path(&file_str) {
            continue;
        }

        file_groups.entry(file_str).or_default().push((id, symbol));
    }

    // Track reported pairs to avoid duplicate diagnostics
    let mut reported_pairs: HashSet<(String, String)> = HashSet::new();

    // Compare all pairs within each file group
    for symbols in file_groups.values() {
        for i in 0..symbols.len() {
            for j in (i + 1)..symbols.len() {
                let (id_a, sym_a) = &symbols[i];
                let (id_b, sym_b) = &symbols[j];

                // Check arity match — require at least one function to have a
                // known signature. Without signatures, arity is unknown and
                // comparing unknown-arity functions produces too many false positives.
                if sym_a.signature.is_none() && sym_b.signature.is_none() {
                    continue;
                }
                let arity_a = extract_arity(sym_a.signature.as_deref());
                let arity_b = extract_arity(sym_b.signature.as_deref());
                if arity_a != arity_b {
                    continue;
                }

                // Get callees sets, excluding self-calls
                let callees_a: HashSet<SymbolId> = graph
                    .callees(*id_a)
                    .into_iter()
                    .filter(|&c| c != *id_a)
                    .collect();
                let callees_b: HashSet<SymbolId> = graph
                    .callees(*id_b)
                    .into_iter()
                    .filter(|&c| c != *id_b)
                    .collect();

                // Skip pairs where both have zero callees (no structural evidence)
                if callees_a.is_empty() && callees_b.is_empty() {
                    continue;
                }

                // Compute Jaccard similarity
                let intersection = callees_a.intersection(&callees_b).count();
                let union = callees_a.union(&callees_b).count();

                // Require a minimum union size for meaningful structural comparison.
                // Two functions sharing 1-2 callees is often coincidental API usage,
                // not structural duplication.
                if union < MIN_CALLEES_FOR_COMPARISON {
                    continue;
                }

                let jaccard = intersection as f64 / union as f64;

                if jaccard < JACCARD_THRESHOLD {
                    continue;
                }

                // Avoid reporting the same pair twice (order-independent)
                let pair_key = if sym_a.qualified_name <= sym_b.qualified_name {
                    (sym_a.qualified_name.clone(), sym_b.qualified_name.clone())
                } else {
                    (sym_b.qualified_name.clone(), sym_a.qualified_name.clone())
                };
                if !reported_pairs.insert(pair_key) {
                    continue;
                }

                // Build evidence with shared callees names
                let shared_names: Vec<String> = callees_a
                    .intersection(&callees_b)
                    .filter_map(|&cid| graph.get_symbol(cid).map(|s| s.name.clone()))
                    .collect();

                let location = format!(
                    "{}:{}",
                    relativize_path(&sym_a.location.file, project_root),
                    sym_a.location.line
                );

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::Duplication,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: format!("{}, {}", sym_a.qualified_name, sym_b.qualified_name),
                    message: format!(
                        "Structural duplication: '{}' and '{}' share {:.0}% of their callees ({}/{} shared)",
                        sym_a.name,
                        sym_b.name,
                        jaccard * 100.0,
                        intersection,
                        union
                    ),
                    evidence: vec![
                        Evidence {
                            observation: format!(
                                "Jaccard similarity: {:.2} ({}/{} callees shared). Shared: [{}]",
                                jaccard,
                                intersection,
                                union,
                                shared_names.join(", ")
                            ),
                            location: Some(location.clone()),
                            context: Some(format!(
                                "arity: {} params each",
                                arity_a
                            )),
                        },
                        Evidence {
                            observation: format!(
                                "'{}' at line {}, '{}' at line {}",
                                sym_a.name,
                                sym_a.location.line,
                                sym_b.name,
                                sym_b.location.line
                            ),
                            location: Some(format!(
                                "{}:{}",
                                relativize_path(&sym_b.location.file, project_root),
                                sym_b.location.line
                            )),
                            context: None,
                        },
                    ],
                    suggestion: format!(
                        "Consider extracting shared logic from '{}' and '{}' into a common function",
                        sym_a.name, sym_b.name
                    ),
                    location,
                });
            }
        }
    }

    diagnostics
}

/// Extract parameter arity from a function signature string.
///
/// Parses signatures like `(param: Type, param2: Type2) -> ReturnType`
/// and counts the number of comma-separated parameters. Returns 0 for
/// empty parameter lists or signatures that can't be parsed.
fn extract_arity(signature: Option<&str>) -> usize {
    let Some(sig) = signature else {
        return 0;
    };
    let sig = sig.trim();
    if !sig.starts_with('(') {
        return 0;
    }

    // Find the matching closing paren
    let inner = if let Some(close) = sig.find(')') {
        &sig[1..close]
    } else {
        return 0;
    };

    let trimmed = inner.trim();
    if trimmed.is_empty() {
        return 0;
    }

    // Count comma-separated parameters
    trimmed.split(',').filter(|p| !p.trim().is_empty()).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ir::{ReferenceKind, Visibility};
    use crate::test_utils::*;
    use std::path::PathBuf;

    // =========================================================================
    // 1. Duplication Pattern Tests
    // =========================================================================

    // -- 1.1 True Positive: Identical callees with matching arity --

    #[test]
    fn test_duplication_true_positive_identical_callees() {
        let mut g = Graph::new();
        let f = "handlers.py";
        let validate = g.add_symbol(make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            10,
        ));
        let notify = g.add_symbol(make_symbol(
            "notify",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));

        let mut process_order = make_symbol(
            "process_order",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        );
        process_order.signature = Some("(order: Order, ctx: Context) -> Result".to_string());
        let po_id = g.add_symbol(process_order);

        let mut process_refund = make_symbol(
            "process_refund",
            SymbolKind::Function,
            Visibility::Public,
            f,
            30,
        );
        process_refund.signature = Some("(refund: Refund, ctx: Context) -> Result".to_string());
        let pr_id = g.add_symbol(process_refund);

        for caller in [po_id, pr_id] {
            add_ref(&mut g, caller, validate, ReferenceKind::Call, f);
            add_ref(&mut g, caller, save, ReferenceKind::Call, f);
            add_ref(&mut g, caller, notify, ReferenceKind::Call, f);
        }

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Expected duplication finding for identical callees, got: {:?}",
            results
        );
        let diag = results
            .iter()
            .find(|d| d.pattern == DiagnosticPattern::Duplication)
            .unwrap();
        assert_eq!(diag.severity, Severity::Warning);
        assert_eq!(diag.confidence, Confidence::Moderate);
        assert!(!diag.evidence.is_empty());
    }

    // -- 1.2 True Negative: Same name different structure --

    #[test]
    fn test_duplication_true_negative_different_callees() {
        let mut g = Graph::new();
        let f = "parsers.py";
        let json_decode = g.add_symbol(make_symbol(
            "json_decode",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let validate_schema = g.add_symbol(make_symbol(
            "validate_schema",
            SymbolKind::Function,
            Visibility::Private,
            f,
            10,
        ));
        let xml_decode = g.add_symbol(make_symbol(
            "xml_decode",
            SymbolKind::Function,
            Visibility::Private,
            f,
            15,
        ));
        let transform_tree = g.add_symbol(make_symbol(
            "transform_tree",
            SymbolKind::Function,
            Visibility::Private,
            f,
            20,
        ));
        let normalize = g.add_symbol(make_symbol(
            "normalize",
            SymbolKind::Function,
            Visibility::Private,
            f,
            25,
        ));

        let mut parse_json = make_symbol(
            "parse_json",
            SymbolKind::Function,
            Visibility::Public,
            f,
            30,
        );
        parse_json.signature = Some("(data) -> dict".to_string());
        let pj_id = g.add_symbol(parse_json);

        let mut parse_xml =
            make_symbol("parse_xml", SymbolKind::Function, Visibility::Public, f, 40);
        parse_xml.signature = Some("(data) -> dict".to_string());
        let px_id = g.add_symbol(parse_xml);

        add_ref(&mut g, pj_id, json_decode, ReferenceKind::Call, f);
        add_ref(&mut g, pj_id, validate_schema, ReferenceKind::Call, f);

        add_ref(&mut g, px_id, xml_decode, ReferenceKind::Call, f);
        add_ref(&mut g, px_id, transform_tree, ReferenceKind::Call, f);
        add_ref(&mut g, px_id, normalize, ReferenceKind::Call, f);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Should NOT trigger duplication for completely different callees"
        );
    }

    // -- 1.3 Adversarial: Threshold boundary --

    #[test]
    fn test_duplication_adversarial_threshold_boundary() {
        let mut g = Graph::new();
        let f = "handlers.py";

        // Shared callees
        let log = g.add_symbol(make_symbol(
            "log",
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
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));
        let notify = g.add_symbol(make_symbol(
            "notify",
            SymbolKind::Function,
            Visibility::Private,
            f,
            4,
        ));
        let audit = g.add_symbol(make_symbol(
            "audit",
            SymbolKind::Function,
            Visibility::Private,
            f,
            5,
        ));
        let check = g.add_symbol(make_symbol(
            "check",
            SymbolKind::Function,
            Visibility::Private,
            f,
            6,
        ));

        // Unique callees
        let format_a = g.add_symbol(make_symbol(
            "format_a",
            SymbolKind::Function,
            Visibility::Private,
            f,
            7,
        ));
        let format_b = g.add_symbol(make_symbol(
            "format_b",
            SymbolKind::Function,
            Visibility::Private,
            f,
            8,
        ));

        let mut handler_a =
            make_symbol("handler_a", SymbolKind::Function, Visibility::Public, f, 20);
        handler_a.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let ha_id = g.add_symbol(handler_a);

        let mut handler_b =
            make_symbol("handler_b", SymbolKind::Function, Visibility::Public, f, 30);
        handler_b.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hb_id = g.add_symbol(handler_b);

        // handler_a calls: log, validate, save, notify, format_a, audit, check (7 total)
        for callee in [log, validate, save, notify, format_a, audit, check] {
            add_ref(&mut g, ha_id, callee, ReferenceKind::Call, f);
        }

        // handler_b calls: log, validate, save, notify, format_b, audit, check (7 total)
        for callee in [log, validate, save, notify, format_b, audit, check] {
            add_ref(&mut g, hb_id, callee, ReferenceKind::Call, f);
        }

        // Jaccard = 6/8 = 0.75 — above threshold
        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Jaccard 0.75 should trigger duplication (threshold 0.7)"
        );
    }

    #[test]
    fn test_duplication_adversarial_below_threshold() {
        let mut g = Graph::new();
        let f = "handlers.py";

        let log = g.add_symbol(make_symbol(
            "log",
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
        let save = g.add_symbol(make_symbol(
            "save",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));
        let format_c = g.add_symbol(make_symbol(
            "format_c",
            SymbolKind::Function,
            Visibility::Private,
            f,
            4,
        ));
        let cache = g.add_symbol(make_symbol(
            "cache",
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
        let render = g.add_symbol(make_symbol(
            "render",
            SymbolKind::Function,
            Visibility::Private,
            f,
            7,
        ));
        let send = g.add_symbol(make_symbol(
            "send",
            SymbolKind::Function,
            Visibility::Private,
            f,
            8,
        ));

        let mut handler_c =
            make_symbol("handler_c", SymbolKind::Function, Visibility::Public, f, 20);
        handler_c.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hc_id = g.add_symbol(handler_c);

        let mut handler_d =
            make_symbol("handler_d", SymbolKind::Function, Visibility::Public, f, 30);
        handler_d.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
        let hd_id = g.add_symbol(handler_d);

        // handler_c calls: log, validate, save, format_c, cache
        for callee in [log, validate, save, format_c, cache] {
            add_ref(&mut g, hc_id, callee, ReferenceKind::Call, f);
        }

        // handler_d calls: log, validate, transform, render, send
        for callee in [log, validate, transform, render, send] {
            add_ref(&mut g, hd_id, callee, ReferenceKind::Call, f);
        }

        // Jaccard = 2/8 = 0.25 — well below threshold
        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Jaccard 0.25 should NOT trigger duplication"
        );
    }

    // -- 1.4 Adversarial: Zero callees --

    #[test]
    fn test_duplication_adversarial_zero_callees() {
        let mut g = Graph::new();
        let f = "getters.py";

        let mut get_name = make_symbol("get_name", SymbolKind::Function, Visibility::Public, f, 5);
        get_name.signature = Some("() -> str".to_string());
        g.add_symbol(get_name);

        let mut get_version = make_symbol(
            "get_version",
            SymbolKind::Function,
            Visibility::Public,
            f,
            10,
        );
        get_version.signature = Some("() -> str".to_string());
        g.add_symbol(get_version);

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Zero-callees functions should NOT trigger duplication"
        );
    }

    // -- 1.5 Adversarial: Different arity --

    #[test]
    fn test_duplication_adversarial_different_arity() {
        let mut g = Graph::new();
        let f = "handlers.py";

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
        let notify = g.add_symbol(make_symbol(
            "notify",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut setup_user = make_symbol(
            "setup_user",
            SymbolKind::Function,
            Visibility::Public,
            f,
            10,
        );
        setup_user.signature = Some("(config, db, logger, flags) -> None".to_string());
        let su_id = g.add_symbol(setup_user);

        let mut handle_event = make_symbol(
            "handle_event",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        );
        handle_event.signature = Some("(event) -> None".to_string());
        let he_id = g.add_symbol(handle_event);

        for callee in [validate, save, notify] {
            add_ref(&mut g, su_id, callee, ReferenceKind::Call, f);
            add_ref(&mut g, he_id, callee, ReferenceKind::Call, f);
        }

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Different arity (4 vs 1) should NOT trigger duplication"
        );
    }

    // -- 1.6 Adversarial: Test file excluded --

    #[test]
    fn test_duplication_adversarial_test_file_excluded() {
        let mut g = Graph::new();
        let f = "test_handlers.py";

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
        let notify = g.add_symbol(make_symbol(
            "notify",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut test_a = make_symbol("helper_a", SymbolKind::Function, Visibility::Public, f, 10);
        test_a.signature = Some("(data: dict, ctx: Context) -> None".to_string());
        let ta_id = g.add_symbol(test_a);

        let mut test_b = make_symbol("helper_b", SymbolKind::Function, Visibility::Public, f, 20);
        test_b.signature = Some("(data: dict, ctx: Context) -> None".to_string());
        let tb_id = g.add_symbol(test_b);

        for callee in [validate, save, notify] {
            add_ref(&mut g, ta_id, callee, ReferenceKind::Call, f);
            add_ref(&mut g, tb_id, callee, ReferenceKind::Call, f);
        }

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            !results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Test files should be excluded from duplication detection"
        );
    }

    // -- 1.7 Adversarial: Self-recursive function --

    #[test]
    fn test_duplication_adversarial_self_recursive() {
        let mut g = Graph::new();
        let f = "handlers.py";

        let process = g.add_symbol(make_symbol(
            "process",
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
        let emit = g.add_symbol(make_symbol(
            "emit",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut traverse = make_symbol("traverse", SymbolKind::Function, Visibility::Public, f, 10);
        traverse.signature = Some("(node) -> None".to_string());
        let tr_id = g.add_symbol(traverse);

        let mut handle = make_symbol("handle", SymbolKind::Function, Visibility::Public, f, 20);
        handle.signature = Some("(item) -> None".to_string());
        let ha_id = g.add_symbol(handle);

        // traverse calls itself + process + validate + emit
        add_ref(&mut g, tr_id, tr_id, ReferenceKind::Call, f);
        add_ref(&mut g, tr_id, process, ReferenceKind::Call, f);
        add_ref(&mut g, tr_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, tr_id, emit, ReferenceKind::Call, f);

        // handle calls process + validate + emit
        add_ref(&mut g, ha_id, process, ReferenceKind::Call, f);
        add_ref(&mut g, ha_id, validate, ReferenceKind::Call, f);
        add_ref(&mut g, ha_id, emit, ReferenceKind::Call, f);

        // With self-call excluded: traverse callees = {process, validate, emit}, handle callees = {process, validate, emit}
        // Jaccard = 1.0, union = 3 (meets min threshold) — should trigger
        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Self-recursive calls should be excluded; remaining callees are identical"
        );
    }

    // -- 1.8 True Positive: Shared callees with matching arity --

    #[test]
    fn test_duplication_true_positive_shared_callees_match() {
        let mut g = Graph::new();
        let f = "serializers.py";

        let to_json = g.add_symbol(make_symbol(
            "to_json",
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
        let format_output = g.add_symbol(make_symbol(
            "format_output",
            SymbolKind::Function,
            Visibility::Private,
            f,
            3,
        ));

        let mut serialize_user = make_symbol(
            "serialize_user",
            SymbolKind::Function,
            Visibility::Public,
            f,
            10,
        );
        serialize_user.signature = Some("(user) -> str".to_string());
        let su_id = g.add_symbol(serialize_user);

        let mut serialize_order = make_symbol(
            "serialize_order",
            SymbolKind::Function,
            Visibility::Public,
            f,
            20,
        );
        serialize_order.signature = Some("(order) -> str".to_string());
        let so_id = g.add_symbol(serialize_order);

        for callee in [to_json, validate, format_output] {
            add_ref(&mut g, su_id, callee, ReferenceKind::Call, f);
            add_ref(&mut g, so_id, callee, ReferenceKind::Call, f);
        }

        let root = PathBuf::from("/project");
        let results = detect(&g, &root);
        assert!(
            results
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::Duplication),
            "Shared callees (>=3) with matching arity should trigger"
        );
    }

    // -- Helper: extract_arity --

    #[test]
    fn test_extract_arity_basic() {
        assert_eq!(extract_arity(Some("(a: int, b: str) -> bool")), 2);
        assert_eq!(extract_arity(Some("(x) -> None")), 1);
        assert_eq!(extract_arity(Some("() -> str")), 0);
        assert_eq!(extract_arity(None), 0);
        assert_eq!(extract_arity(Some("")), 0);
        assert_eq!(extract_arity(Some("(a, b, c, d) -> None")), 4);
    }
}
