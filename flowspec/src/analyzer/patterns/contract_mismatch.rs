// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Contract mismatch detection — interface says one thing, implementation says another.
//!
//! Two detection phases with distinct confidence levels:
//!
//! **Phase 1 (HIGH confidence): Python decorator contract violations.**
//! Language rules, not heuristics. Detects:
//! - `@staticmethod` with `self` as first parameter
//! - `@classmethod` without `cls`/`klass` as first parameter
//! - `@property` with parameters beyond `self`
//!
//! **Phase 2 (MODERATE confidence): Cross-file same-name signature inconsistency.**
//! Functions with the same name defined in different files but with different
//! parameter counts. Heuristic — may be intentional overloading. Excludes
//! dunder methods, test functions, and variadic signatures (`*args`, `**kwargs`).
//!
//! Does NOT use `is_excluded_symbol()` as a blanket pre-filter for Phase 1
//! because decorator contract violations apply regardless of test context.
//! Phase 2 uses targeted exclusions (dunders, test functions, imports).
//!
//! Serde annotation mismatch, call-site arity, and type annotation validation
//! are deferred — they require parser-level changes not yet available.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::relativize_path;
use crate::graph::Graph;
use crate::parser::ir::SymbolKind;

/// Detect contract mismatches in the analysis graph.
///
/// Runs two detection phases:
/// 1. Python decorator contract violations (HIGH confidence)
/// 2. Cross-file same-name signature inconsistency (MODERATE confidence)
///
/// The `project_root` path is used to produce relative file paths in
/// diagnostic locations.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();

    detect_decorator_violations(graph, project_root, &mut diagnostics);
    detect_cross_file_arity_mismatch(graph, project_root, &mut diagnostics);

    diagnostics
}

/// Phase 1: Python decorator contract violations.
///
/// Walks all symbols with `@staticmethod`, `@classmethod`, or `@property`
/// annotations and validates their signatures against Python language rules.
/// These are deterministic checks — HIGH confidence.
fn detect_decorator_violations(
    graph: &Graph,
    project_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    for (_id, symbol) in graph.all_symbols() {
        // Only check methods with signatures and decorator annotations
        if symbol.kind != SymbolKind::Method && symbol.kind != SymbolKind::Function {
            continue;
        }

        let signature = match &symbol.signature {
            Some(sig) if !sig.is_empty() => sig,
            _ => continue,
        };

        let params = parse_params(signature);

        let location = format!(
            "{}:{}",
            relativize_path(&symbol.location.file, project_root),
            symbol.location.line
        );

        // Check @staticmethod: should NOT have self as first param
        if symbol.annotations.contains(&"staticmethod".to_string()) {
            if let Some(first) = params.first() {
                if first == "self" {
                    diagnostics.push(Diagnostic {
                        id: String::new(),
                        pattern: DiagnosticPattern::ContractMismatch,
                        severity: Severity::Critical,
                        confidence: Confidence::High,
                        entity: symbol.name.clone(),
                        message: format!(
                            "Contract mismatch: @staticmethod '{}' has 'self' as first parameter",
                            symbol.name
                        ),
                        evidence: vec![Evidence {
                            observation: format!(
                                "@staticmethod '{}' has signature '{}' with 'self' as first parameter. \
                                 Static methods must not receive an instance reference.",
                                symbol.name, signature
                            ),
                            location: Some(location.clone()),
                            context: Some(
                                "Python language rule: @staticmethod removes the implicit \
                                 instance binding"
                                    .to_string(),
                            ),
                        }],
                        suggestion: format!(
                            "Remove 'self' from the parameter list of '{}', or remove \
                             the @staticmethod decorator if this method needs instance access.",
                            symbol.name
                        ),
                        location: location.clone(),
                    });
                }
            }
        }

        // Check @classmethod: first param should be cls/klass
        if symbol.annotations.contains(&"classmethod".to_string()) {
            if let Some(first) = params.first() {
                let is_cls_like =
                    first == "cls" || first == "klass" || first == "class_" || first == "mcls";
                if !is_cls_like {
                    diagnostics.push(Diagnostic {
                        id: String::new(),
                        pattern: DiagnosticPattern::ContractMismatch,
                        severity: Severity::Critical,
                        confidence: Confidence::High,
                        entity: symbol.name.clone(),
                        message: format!(
                            "Contract mismatch: @classmethod '{}' has '{}' instead of 'cls' \
                             as first parameter",
                            symbol.name, first
                        ),
                        evidence: vec![Evidence {
                            observation: format!(
                                "@classmethod '{}' has signature '{}'. First parameter is \
                                 '{}', expected 'cls' or equivalent.",
                                symbol.name, signature, first
                            ),
                            location: Some(location.clone()),
                            context: Some(
                                "Python convention: @classmethod's first parameter should be \
                                 'cls' (or 'klass', 'class_', 'mcls')"
                                    .to_string(),
                            ),
                        }],
                        suggestion: format!(
                            "Rename the first parameter of '{}' from '{}' to 'cls', \
                             or remove the @classmethod decorator.",
                            symbol.name, first
                        ),
                        location: location.clone(),
                    });
                }
            }
        }

        // Check @property: should only have self, no extra params
        if symbol.annotations.contains(&"property".to_string()) {
            // Count params beyond 'self'
            let non_self_params: Vec<&str> = params
                .iter()
                .filter(|p| p.as_str() != "self")
                .map(|p| p.as_str())
                .collect();

            if !non_self_params.is_empty() {
                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::ContractMismatch,
                    severity: Severity::Warning,
                    confidence: Confidence::High,
                    entity: symbol.name.clone(),
                    message: format!(
                        "Contract mismatch: @property '{}' has parameters beyond 'self'",
                        symbol.name
                    ),
                    evidence: vec![Evidence {
                        observation: format!(
                            "@property '{}' has signature '{}'. Property getters \
                             must take only 'self', but found extra parameters: {}",
                            symbol.name,
                            signature,
                            non_self_params.join(", ")
                        ),
                        location: Some(location.clone()),
                        context: Some(
                            "Python descriptor protocol: @property getter takes only 'self'"
                                .to_string(),
                        ),
                    }],
                    suggestion: format!(
                        "Remove extra parameters from property getter '{}'. \
                         If additional arguments are needed, use a regular method instead.",
                        symbol.name
                    ),
                    location,
                });
            }
        }
    }
}

/// Determine the language family from a file path's extension.
///
/// Returns a language identifier string based on file extension:
/// - `"python"` for `.py`
/// - `"rust"` for `.rs`
/// - `"javascript"` for `.js`, `.jsx`, `.ts`, `.tsx`, `.mjs`, `.cjs`
/// - `"unknown"` for unrecognized or missing extensions
fn language_from_path(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("py") => "python",
        Some("rs") => "rust",
        Some("js") | Some("jsx") | Some("ts") | Some("tsx") | Some("mjs") | Some("cjs") => {
            "javascript"
        }
        _ => "unknown",
    }
}

/// Phase 2: Cross-file same-name functions with different parameter counts.
///
/// Groups all Function/Method symbols by `(language, name)`, then checks for
/// groups where definitions in different files have different parameter counts.
/// Excludes dunders, test functions, imports, and variadic signatures.
///
/// Language-aware scoping prevents false positives:
/// - Cross-language comparisons are eliminated (Python vs Rust vs JS).
/// - For Rust, different files are different modules — cross-file comparisons
///   are excluded since same-name functions in different Rust modules are normal
///   architecture (e.g., `make_symbol` in `parser/python.rs` vs `parser/rust.rs`).
/// - For Python and JavaScript, cross-file comparison is preserved — that's the
///   designed use case for detecting inconsistent interfaces.
/// - Unknown/unrecognized extensions are excluded from Phase 2 entirely.
///
/// MODERATE confidence — may be intentional overloading.
fn detect_cross_file_arity_mismatch(
    graph: &Graph,
    project_root: &Path,
    diagnostics: &mut Vec<Diagnostic>,
) {
    // Group symbols by (language, name) to prevent cross-language comparisons
    let mut by_lang_name: HashMap<(&str, &str), Vec<(&crate::parser::ir::Symbol, usize)>> =
        HashMap::new();

    for (_id, symbol) in graph.all_symbols() {
        // Only functions and methods
        if symbol.kind != SymbolKind::Function && symbol.kind != SymbolKind::Method {
            continue;
        }

        // Skip dunders
        if symbol.name.starts_with("__") && symbol.name.ends_with("__") {
            continue;
        }

        // Skip test functions
        if symbol.name.starts_with("test_") {
            continue;
        }

        // Skip imports
        if symbol.annotations.contains(&"import".to_string()) {
            continue;
        }

        // Must have a signature to compare
        let signature = match &symbol.signature {
            Some(sig) if !sig.is_empty() => sig,
            _ => continue,
        };

        // Skip variadic signatures (*args, **kwargs)
        if is_variadic(signature) {
            continue;
        }

        // Determine language — skip unknown extensions entirely
        let lang = language_from_path(&symbol.location.file);
        if lang == "unknown" {
            continue;
        }

        let param_count = count_params(signature);

        by_lang_name
            .entry((lang, &symbol.name))
            .or_default()
            .push((symbol, param_count));
    }

    // Check each group for arity mismatches
    for ((lang, name), symbols) in &by_lang_name {
        if symbols.len() < 2 {
            continue;
        }

        // Rust-specific exclusion: different files = different modules.
        // In Rust, each file defines a module. Same-name functions in different
        // Rust modules are normal architecture, not contract mismatches.
        if *lang == "rust" {
            let files: HashSet<&Path> = symbols
                .iter()
                .map(|(s, _)| s.location.file.as_path())
                .collect();
            if files.len() > 1 {
                continue;
            }
        }

        // Check if there are different parameter counts
        let first_count = symbols[0].1;
        let has_mismatch = symbols.iter().any(|(_, count)| *count != first_count);

        if !has_mismatch {
            continue;
        }

        // Collect the distinct arities with their locations
        let mut arity_locations: Vec<(usize, String)> = Vec::new();
        for (sym, count) in symbols {
            let loc = format!(
                "{}:{}",
                relativize_path(&sym.location.file, project_root),
                sym.location.line
            );
            arity_locations.push((*count, loc));
        }

        // Use the first symbol's location as primary
        let primary_location = arity_locations[0].1.clone();

        let arity_details: Vec<String> = arity_locations
            .iter()
            .map(|(count, loc)| format!("{} params at {}", count, loc))
            .collect();

        diagnostics.push(Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::ContractMismatch,
            severity: Severity::Warning,
            confidence: Confidence::Moderate,
            entity: name.to_string(),
            message: format!(
                "Contract mismatch: function '{}' defined with different parameter counts \
                 across files",
                name
            ),
            evidence: vec![Evidence {
                observation: format!(
                    "Function '{}' has inconsistent signatures: {}",
                    name,
                    arity_details.join(", ")
                ),
                location: Some(primary_location.clone()),
                context: Some(
                    "Same-name functions with different parameter counts across files \
                     create contract ambiguity"
                        .to_string(),
                ),
            }],
            suggestion: format!(
                "Verify that all definitions of '{}' should have different parameter \
                 counts. If they serve different purposes, consider distinct names. \
                 If one is outdated, update its signature.",
                name
            ),
            location: primary_location,
        });
    }
}

/// Parse parameter names from a signature string.
///
/// Handles:
/// - Type annotations: `(data: Dict[str, int])` → `["data"]`
/// - Default values: `(x=5)` → `["x"]`
/// - Return type: `(self, x) -> int` → `["self", "x"]`
/// - Nested brackets: commas inside `[]` or `()` are not separators
/// - Variadic: `(*args, **kwargs)` → `["*args", "**kwargs"]`
/// - Empty: `()` → `[]`
fn parse_params(signature: &str) -> Vec<String> {
    // Find the parameter section between outermost parens
    let start = match signature.find('(') {
        Some(i) => i + 1,
        None => return Vec::new(),
    };

    // Find the matching close paren, accounting for nesting
    let mut depth = 1;
    let mut end = start;
    for (i, ch) in signature[start..].char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                depth -= 1;
                if depth == 0 {
                    end = start + i;
                    break;
                }
            }
            _ => {}
        }
    }

    let param_section = &signature[start..end];

    if param_section.trim().is_empty() {
        return Vec::new();
    }

    // Split by commas, respecting bracket nesting
    let mut params = Vec::new();
    let mut current = String::new();
    let mut bracket_depth = 0;

    for ch in param_section.chars() {
        match ch {
            '[' | '(' => {
                bracket_depth += 1;
                current.push(ch);
            }
            ']' | ')' => {
                bracket_depth -= 1;
                current.push(ch);
            }
            ',' if bracket_depth == 0 => {
                let param = extract_param_name(current.trim());
                if !param.is_empty() {
                    params.push(param);
                }
                current.clear();
            }
            _ => {
                current.push(ch);
            }
        }
    }

    // Last parameter
    let param = extract_param_name(current.trim());
    if !param.is_empty() {
        params.push(param);
    }

    params
}

/// Extract the parameter name from a parameter string.
///
/// Handles type annotations (`x: int` → `x`) and default values (`x=5` → `x`).
/// Preserves `*` and `**` prefixes for variadic detection.
fn extract_param_name(param: &str) -> String {
    let trimmed = param.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Handle type annotation: take everything before ':'
    let name_part = if let Some(colon_idx) = trimmed.find(':') {
        &trimmed[..colon_idx]
    } else if let Some(eq_idx) = trimmed.find('=') {
        &trimmed[..eq_idx]
    } else {
        trimmed
    };

    name_part.trim().to_string()
}

/// Count the number of parameters in a signature string.
///
/// Handles nested brackets in type annotations. `self` is counted as a
/// parameter for raw counting (Phase 2 compares raw counts across definitions).
fn count_params(signature: &str) -> usize {
    parse_params(signature).len()
}

/// Check if a signature contains variadic parameters (*args or **kwargs).
fn is_variadic(signature: &str) -> bool {
    let params = parse_params(signature);
    params
        .iter()
        .any(|p| p.starts_with('*') || p.starts_with("**"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::parser::ir::*;
    use crate::test_utils::*;

    // =========================================================================
    // Graph builders for contract_mismatch tests
    // =========================================================================

    /// Contract mismatch fixture: decorator violations and cross-file arity mismatches.
    fn build_contract_mismatch_graph() -> Graph {
        let mut g = Graph::new();
        let f = "service.py";

        // Phase 1: Decorator contract violations (Python)

        // BAD: @staticmethod with self as first param
        let mut bad_static =
            make_symbol("bad_static", SymbolKind::Method, Visibility::Public, f, 5);
        bad_static.annotations.push("staticmethod".to_string());
        bad_static.signature = Some("(self, x)".to_string());
        g.add_symbol(bad_static);

        // BAD: @classmethod with self instead of cls
        let mut bad_classmethod = make_symbol(
            "bad_classmethod",
            SymbolKind::Method,
            Visibility::Public,
            f,
            10,
        );
        bad_classmethod.annotations.push("classmethod".to_string());
        bad_classmethod.signature = Some("(self, x)".to_string());
        g.add_symbol(bad_classmethod);

        // BAD: @property with extra params beyond self
        let mut bad_property = make_symbol(
            "bad_property",
            SymbolKind::Method,
            Visibility::Public,
            f,
            15,
        );
        bad_property.annotations.push("property".to_string());
        bad_property.signature = Some("(self, extra)".to_string());
        g.add_symbol(bad_property);

        // Phase 2: Cross-file same-name functions with different arity
        let mut process_a = make_symbol(
            "process_data",
            SymbolKind::Function,
            Visibility::Public,
            "module_a.py",
            1,
        );
        process_a.signature = Some("(data, config)".to_string());
        g.add_symbol(process_a);

        let mut process_b = make_symbol(
            "process_data",
            SymbolKind::Function,
            Visibility::Public,
            "module_b.py",
            1,
        );
        process_b.signature = Some("(data)".to_string());
        g.add_symbol(process_b);

        g
    }

    /// Clean code — no contract violations.
    fn build_clean_contract_graph() -> Graph {
        let mut g = Graph::new();
        let f = "clean_service.py";

        // GOOD: @staticmethod without self
        let mut good_static =
            make_symbol("good_static", SymbolKind::Method, Visibility::Public, f, 5);
        good_static.annotations.push("staticmethod".to_string());
        good_static.signature = Some("(x, y)".to_string());
        g.add_symbol(good_static);

        // GOOD: @classmethod with cls
        let mut good_classmethod = make_symbol(
            "good_classmethod",
            SymbolKind::Method,
            Visibility::Public,
            f,
            10,
        );
        good_classmethod.annotations.push("classmethod".to_string());
        good_classmethod.signature = Some("(cls, x)".to_string());
        g.add_symbol(good_classmethod);

        // GOOD: @property with only self
        let mut good_property = make_symbol(
            "good_property",
            SymbolKind::Method,
            Visibility::Public,
            f,
            15,
        );
        good_property.annotations.push("property".to_string());
        good_property.signature = Some("(self)".to_string());
        g.add_symbol(good_property);

        // Two same-name functions with SAME arity — not a violation
        let mut fn_a = make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "mod_a.py",
            1,
        );
        fn_a.signature = Some("(x, y)".to_string());
        g.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "mod_b.py",
            1,
        );
        fn_b.signature = Some("(a, b)".to_string());
        g.add_symbol(fn_b);

        g
    }

    // =========================================================================
    // T1: True Positive — @staticmethod with self parameter
    // =========================================================================

    #[test]
    fn test_contract_mismatch_fires_on_staticmethod_with_self() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let static_diag = diagnostics.iter().find(|d| d.entity.contains("bad_static"));
        assert!(
            static_diag.is_some(),
            "contract_mismatch must fire on @staticmethod with self param. Got entities: {:?}",
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );

        let diag = static_diag.unwrap();
        assert_eq!(diag.pattern, DiagnosticPattern::ContractMismatch);
        assert_eq!(
            diag.severity,
            Severity::Critical,
            "contract_mismatch spec severity is CRITICAL, got {:?}",
            diag.severity
        );
        assert_eq!(
            diag.confidence,
            Confidence::High,
            "Decorator contract violation is a Python language rule — must be HIGH confidence"
        );
        assert!(!diag.evidence.is_empty(), "Must include evidence");
        assert!(
            !diag.suggestion.is_empty(),
            "Must include actionable suggestion"
        );
    }

    // =========================================================================
    // T2: True Positive — @classmethod without cls
    // =========================================================================

    #[test]
    fn test_contract_mismatch_fires_on_classmethod_without_cls() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let cls_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("bad_classmethod"));
        assert!(
            cls_diag.is_some(),
            "contract_mismatch must fire on @classmethod with self instead of cls"
        );

        let diag = cls_diag.unwrap();
        assert_eq!(diag.pattern, DiagnosticPattern::ContractMismatch);
        assert_eq!(diag.severity, Severity::Critical);
        assert_eq!(diag.confidence, Confidence::High);
    }

    // =========================================================================
    // T3: True Positive — @property with extra parameters
    // =========================================================================

    #[test]
    fn test_contract_mismatch_fires_on_property_with_extra_params() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let prop_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("bad_property"));
        assert!(
            prop_diag.is_some(),
            "contract_mismatch must fire on @property with params beyond self"
        );

        let diag = prop_diag.unwrap();
        assert_eq!(diag.pattern, DiagnosticPattern::ContractMismatch);
        assert_eq!(
            diag.confidence,
            Confidence::High,
            "@property contract is a Python language rule"
        );
    }

    // =========================================================================
    // T4: True Positive — Cross-file same-name different arity
    // =========================================================================

    #[test]
    fn test_contract_mismatch_fires_on_cross_file_arity_mismatch() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let arity_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("process_data") && d.confidence == Confidence::Moderate);
        assert!(
            arity_diag.is_some(),
            "contract_mismatch must detect cross-file same-name functions with different arity. \
             Got entities: {:?}",
            diagnostics
                .iter()
                .filter(|d| d.entity.contains("process_data"))
                .map(|d| (&d.entity, &d.confidence))
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T5: True Negative — Clean code produces no findings
    // =========================================================================

    #[test]
    fn test_contract_mismatch_clean_code_no_findings() {
        let graph = build_clean_contract_graph();
        let diagnostics = detect(&graph, Path::new(""));

        assert!(
            diagnostics.is_empty(),
            "contract_mismatch must NOT fire on clean code. Got {} findings: {:?}",
            diagnostics.len(),
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T6: Confidence Calibration — Decorator=HIGH, Arity=MODERATE
    // =========================================================================

    #[test]
    fn test_contract_mismatch_confidence_calibration() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        // Decorator violations must be HIGH
        for name in &["bad_static", "bad_classmethod", "bad_property"] {
            let diag = diagnostics.iter().find(|d| d.entity.contains(name));
            if let Some(d) = diag {
                assert_eq!(
                    d.confidence,
                    Confidence::High,
                    "Decorator contract violation for {} must be HIGH confidence, got {:?}",
                    name,
                    d.confidence
                );
            }
        }

        // Cross-file arity mismatch must be MODERATE
        let arity_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("process_data"))
            .collect();
        for d in &arity_diags {
            assert_eq!(
                d.confidence,
                Confidence::Moderate,
                "Cross-file arity mismatch must be MODERATE confidence, got {:?}",
                d.confidence
            );
        }
    }

    // =========================================================================
    // T7: Pattern Registration Guard
    // =========================================================================

    #[test]
    fn test_contract_mismatch_runs_through_run_all_patterns() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = crate::analyzer::patterns::run_all_patterns(&graph, Path::new(""));

        let contract_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.pattern == DiagnosticPattern::ContractMismatch)
            .collect();

        assert!(
            !contract_findings.is_empty(),
            "contract_mismatch must be registered in run_all_patterns(). \
             If this fails, add contract_mismatch::detect() to the pattern_results vec \
             in patterns/mod.rs."
        );
    }

    // =========================================================================
    // T8: Adversarial — @staticmethod without self is NOT flagged
    // =========================================================================

    #[test]
    fn test_contract_mismatch_staticmethod_without_self_is_clean() {
        let mut graph = Graph::new();
        let mut method = make_symbol(
            "compute",
            SymbolKind::Method,
            Visibility::Public,
            "math.py",
            1,
        );
        method.annotations.push("staticmethod".to_string());
        method.signature = Some("(x, y)".to_string());
        graph.add_symbol(method);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "@staticmethod without self must not trigger contract_mismatch"
        );
    }

    // =========================================================================
    // T9: Adversarial — @classmethod with cls variant names
    // =========================================================================

    #[test]
    fn test_contract_mismatch_classmethod_accepts_cls_variants() {
        let mut graph = Graph::new();

        for (name, param) in [("method_a", "(cls, x)"), ("method_b", "(klass, x)")] {
            let mut method = make_symbol(name, SymbolKind::Method, Visibility::Public, "svc.py", 1);
            method.annotations.push("classmethod".to_string());
            method.signature = Some(param.to_string());
            graph.add_symbol(method);
        }

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "cls/klass variants must not trigger contract_mismatch. Got: {:?}",
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T10: Adversarial — Signature with nested type annotations
    // =========================================================================

    #[test]
    fn test_contract_mismatch_handles_complex_type_signatures() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        );
        fn_a.signature = Some("(data: Dict[str, int], config: Optional[Config])".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        );
        fn_b.signature = Some("(data: Dict[str, int])".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));

        // Should detect the arity difference (2 params vs 1 param)
        let mismatch = diagnostics.iter().find(|d| d.entity.contains("transform"));
        assert!(
            mismatch.is_some(),
            "Must detect cross-file arity mismatch even with complex type annotations. \
             The signature parser must handle commas inside brackets correctly."
        );
    }

    // =========================================================================
    // T11: Adversarial — Empty graph produces no findings, no panic
    // =========================================================================

    #[test]
    fn test_contract_mismatch_empty_graph_no_panic() {
        let graph = Graph::new();
        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Empty graph must produce zero findings"
        );
    }

    // =========================================================================
    // T12: Adversarial — Symbol with None signature is silently skipped
    // =========================================================================

    #[test]
    fn test_contract_mismatch_none_signature_skipped() {
        let mut graph = Graph::new();
        let mut method = make_symbol("no_sig", SymbolKind::Method, Visibility::Public, "x.py", 1);
        method.annotations.push("staticmethod".to_string());
        method.signature = None;
        graph.add_symbol(method);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Symbol with None signature must not trigger contract_mismatch"
        );
    }

    // =========================================================================
    // T13: Adversarial — *args, **kwargs signatures
    // =========================================================================

    #[test]
    fn test_contract_mismatch_args_kwargs_not_false_positive() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "handler",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        );
        fn_a.signature = Some("(data)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "handler",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        );
        fn_b.signature = Some("(*args, **kwargs)".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));
        let handler_diags: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("handler"))
            .collect();
        assert!(
            handler_diags.is_empty(),
            "*args/**kwargs functions must not trigger cross-file arity mismatch. \
             Got: {:?}",
            handler_diags.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T14: Adversarial — Test functions and dunders excluded from Phase 2
    // =========================================================================

    #[test]
    fn test_contract_mismatch_excludes_dunders_and_test_functions() {
        let mut graph = Graph::new();

        // Two __init__ methods with different arity — normal, not a violation
        let mut init_a = make_symbol(
            "__init__",
            SymbolKind::Method,
            Visibility::Public,
            "a.py",
            1,
        );
        init_a.signature = Some("(self, x)".to_string());
        graph.add_symbol(init_a);

        let mut init_b = make_symbol(
            "__init__",
            SymbolKind::Method,
            Visibility::Public,
            "b.py",
            1,
        );
        init_b.signature = Some("(self, x, y, z)".to_string());
        graph.add_symbol(init_b);

        // Two test_ functions with different arity — normal
        let mut test_a = make_symbol(
            "test_process",
            SymbolKind::Function,
            Visibility::Private,
            "test_a.py",
            1,
        );
        test_a.signature = Some("()".to_string());
        graph.add_symbol(test_a);

        let mut test_b = make_symbol(
            "test_process",
            SymbolKind::Function,
            Visibility::Private,
            "test_b.py",
            1,
        );
        test_b.signature = Some("(fixture)".to_string());
        graph.add_symbol(test_b);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Dunders and test functions must be excluded from Phase 2 arity checks. \
             Got: {:?}",
            diagnostics.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T15: Pattern count regression guard
    // =========================================================================

    #[test]
    fn test_pattern_count_at_least_nine() {
        let contract_graph = build_contract_mismatch_graph();
        let filter = crate::analyzer::patterns::PatternFilter {
            patterns: Some(vec![DiagnosticPattern::ContractMismatch]),
            ..Default::default()
        };
        let diagnostics =
            crate::analyzer::patterns::run_patterns(&contract_graph, &filter, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "ContractMismatch must be registered and produce findings when filtered explicitly"
        );
    }

    // =========================================================================
    // T18: Regression — Decorator violations detected in test files
    // =========================================================================

    #[test]
    fn test_contract_mismatch_decorator_violations_detected_in_test_files() {
        let mut graph = Graph::new();
        let mut method = make_symbol(
            "bad_static",
            SymbolKind::Method,
            Visibility::Public,
            "test_service.py",
            5,
        );
        method.annotations.push("staticmethod".to_string());
        method.signature = Some("(self, x)".to_string());
        graph.add_symbol(method);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            !diagnostics.is_empty(),
            "Decorator contract violations must be detected even in test files. \
             Phase 1 must NOT use is_excluded_symbol() as blanket pre-filter."
        );
    }

    // =========================================================================
    // Signature parsing unit tests
    // =========================================================================

    #[test]
    fn test_parse_params_simple() {
        assert_eq!(parse_params("(x, y, z)"), vec!["x", "y", "z"]);
    }

    #[test]
    fn test_parse_params_with_self() {
        assert_eq!(parse_params("(self, x)"), vec!["self", "x"]);
    }

    #[test]
    fn test_parse_params_empty() {
        let result: Vec<String> = Vec::new();
        assert_eq!(parse_params("()"), result);
    }

    #[test]
    fn test_parse_params_type_annotations() {
        assert_eq!(
            parse_params("(data: Dict[str, int], config: Optional[Config])"),
            vec!["data", "config"]
        );
    }

    #[test]
    fn test_parse_params_with_return_type() {
        assert_eq!(parse_params("(self, x) -> int"), vec!["self", "x"]);
    }

    #[test]
    fn test_parse_params_variadic() {
        assert_eq!(parse_params("(*args, **kwargs)"), vec!["*args", "**kwargs"]);
    }

    #[test]
    fn test_parse_params_nested_brackets() {
        assert_eq!(
            parse_params("(fn: Callable[[int], str], x: int)"),
            vec!["fn", "x"]
        );
    }

    #[test]
    fn test_count_params_simple() {
        assert_eq!(count_params("(a, b, c)"), 3);
    }

    #[test]
    fn test_count_params_complex_types() {
        assert_eq!(
            count_params("(data: Dict[str, int], config: Optional[Config])"),
            2
        );
    }

    #[test]
    fn test_is_variadic_true() {
        assert!(is_variadic("(*args, **kwargs)"));
        assert!(is_variadic("(x, *args)"));
    }

    #[test]
    fn test_is_variadic_false() {
        assert!(!is_variadic("(x, y)"));
        assert!(!is_variadic("(self)"));
    }

    // =========================================================================
    // T19: Regression — Rust cross-module same-name different arity must NOT fire
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_fp_rust_cross_module_same_name() {
        let mut graph = Graph::new();

        let mut make_sym_py = make_symbol(
            "make_symbol",
            SymbolKind::Function,
            Visibility::Public,
            "parser/python.rs",
            134,
        );
        make_sym_py.signature = Some("(name: &str, kind: SymbolKind, node: &Node)".to_string());
        graph.add_symbol(make_sym_py);

        let mut make_sym_rs = make_symbol(
            "make_symbol",
            SymbolKind::Function,
            Visibility::Public,
            "parser/rust.rs",
            148,
        );
        make_sym_rs.signature = Some("(name: &str, kind: SymbolKind)".to_string());
        graph.add_symbol(make_sym_rs);

        let diagnostics = detect(&graph, Path::new(""));

        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::Moderate)
            .filter(|d| d.entity.contains("make_symbol"))
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Phase 2 must NOT fire on same-name functions in different Rust modules. \
             make_symbol in parser/python.rs and parser/rust.rs are different module \
             implementations, not contract mismatches. Got {} findings: {:?}",
            phase2_findings.len(),
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T20: Regression — extract_visibility cross-module FP
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_fp_rust_extract_visibility_cross_module() {
        let mut graph = Graph::new();

        let mut vis_py = make_symbol(
            "extract_visibility",
            SymbolKind::Function,
            Visibility::Public,
            "parser/python.rs",
            50,
        );
        vis_py.signature = Some("(node: &Node, source: &str)".to_string());
        graph.add_symbol(vis_py);

        let mut vis_rs = make_symbol(
            "extract_visibility",
            SymbolKind::Function,
            Visibility::Public,
            "parser/rust.rs",
            60,
        );
        vis_rs.signature = Some("(node: &Node, source: &str, parent: Option<&Node>)".to_string());
        graph.add_symbol(vis_rs);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "extract_visibility in parser/python.rs (2 params) and parser/rust.rs (3 params) \
             are different language adapters — Phase 2 must not fire. Got: {:?}",
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T21: Cross-language exclusion — Python vs Rust must NOT compare
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_fp_cross_language_python_rust() {
        let mut graph = Graph::new();

        let mut py_fn = make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            10,
        );
        py_fn.signature = Some("(data)".to_string());
        graph.add_symbol(py_fn);

        let mut rs_fn = make_symbol(
            "transform",
            SymbolKind::Function,
            Visibility::Public,
            "utils.rs",
            10,
        );
        rs_fn.signature = Some("(data: &str, config: &Config)".to_string());
        graph.add_symbol(rs_fn);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Cross-language comparison (Python vs Rust) must be excluded from Phase 2. \
             Different languages naturally have different function signatures. Got: {:?}",
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T22: Cross-language exclusion — Python vs JavaScript must NOT compare
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_fp_cross_language_python_js() {
        let mut graph = Graph::new();

        let mut py_fn = make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Public,
            "validator.py",
            1,
        );
        py_fn.signature = Some("(self, data, schema)".to_string());
        graph.add_symbol(py_fn);

        let mut js_fn = make_symbol(
            "validate",
            SymbolKind::Function,
            Visibility::Public,
            "validator.js",
            1,
        );
        js_fn.signature = Some("(data)".to_string());
        graph.add_symbol(js_fn);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Python vs JavaScript same-name functions must not trigger Phase 2. Got: {:?}",
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T23: True positive preserved — Python cross-file arity mismatch MUST fire
    // =========================================================================

    #[test]
    fn test_contract_mismatch_python_cross_file_still_fires() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "handler_a.py",
            1,
        );
        fn_a.signature = Some("(data)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "handler_b.py",
            1,
        );
        fn_b.signature = Some("(data, extra)".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("process") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            !phase2_findings.is_empty(),
            "Phase 2 MUST still fire for Python cross-file arity mismatch. \
             process(data) in handler_a.py vs process(data, extra) in handler_b.py \
             is a genuine contract mismatch that Phase 2 was designed to catch."
        );
    }

    // =========================================================================
    // T24: Phase 1 unchanged — decorator violations still fire after FP fix
    // =========================================================================

    #[test]
    fn test_contract_mismatch_phase1_unaffected_by_phase2_fix() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let phase1_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.confidence == Confidence::High)
            .collect();

        assert!(
            phase1_findings.len() >= 3,
            "Phase 1 must still detect all 3 decorator violations (staticmethod with self, \
             classmethod without cls, property with extra params). Got {} HIGH-confidence findings: {:?}",
            phase1_findings.len(),
            phase1_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );

        let entities: Vec<&str> = phase1_findings.iter().map(|d| d.entity.as_str()).collect();
        assert!(entities.iter().any(|e| e.contains("bad_static")));
        assert!(entities.iter().any(|e| e.contains("bad_classmethod")));
        assert!(entities.iter().any(|e| e.contains("bad_property")));
    }

    // =========================================================================
    // T25: Adversarial — same name in 3 languages, no crash, no FP
    // =========================================================================

    #[test]
    fn test_contract_mismatch_three_languages_same_name_no_crash() {
        let mut graph = Graph::new();

        let mut py_fn = make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "utils.py",
            1,
        );
        py_fn.signature = Some("(x)".to_string());
        graph.add_symbol(py_fn);

        let mut rs_fn = make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "utils.rs",
            1,
        );
        rs_fn.signature = Some("(x: i32, y: i32)".to_string());
        graph.add_symbol(rs_fn);

        let mut js_fn = make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "utils.js",
            1,
        );
        js_fn.signature = Some("()".to_string());
        graph.add_symbol(js_fn);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("helper") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Same-name function in 3 different languages (py/rs/js) must not crash \
             and must produce zero Phase 2 FPs. Each language is isolated. Got: {:?}",
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T26: Adversarial — .ts, .tsx, .jsx, .mjs, .cjs extensions handled
    // =========================================================================

    #[test]
    fn test_contract_mismatch_typescript_extensions_isolated() {
        let mut graph = Graph::new();

        let mut ts_fn = make_symbol(
            "render",
            SymbolKind::Function,
            Visibility::Public,
            "component.tsx",
            1,
        );
        ts_fn.signature = Some("(props: Props)".to_string());
        graph.add_symbol(ts_fn);

        let mut py_fn = make_symbol(
            "render",
            SymbolKind::Function,
            Visibility::Public,
            "template.py",
            1,
        );
        py_fn.signature = Some("(context, template_name, extra)".to_string());
        graph.add_symbol(py_fn);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("render") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            ".tsx extension must be recognized as JavaScript/TypeScript family \
             and isolated from Python comparisons. Got: {:?}",
            phase2_findings
                .iter()
                .map(|d| &d.entity)
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T27: Edge case — unknown extension excluded from Phase 2
    // =========================================================================

    #[test]
    fn test_contract_mismatch_unknown_extension_no_crash() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "compute",
            SymbolKind::Function,
            Visibility::Public,
            "module.xyz",
            1,
        );
        fn_a.signature = Some("(a)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "compute",
            SymbolKind::Function,
            Visibility::Public,
            "other.xyz",
            1,
        );
        fn_b.signature = Some("(a, b, c)".to_string());
        graph.add_symbol(fn_b);

        // Should not panic. Unknown extensions are excluded from Phase 2.
        let _diagnostics = detect(&graph, Path::new(""));
    }

    // =========================================================================
    // T28: Edge case — file without extension (e.g., Makefile, Dockerfile)
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_extension_no_crash() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "build",
            SymbolKind::Function,
            Visibility::Public,
            "Makefile",
            1,
        );
        fn_a.signature = Some("(target)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "build",
            SymbolKind::Function,
            Visibility::Public,
            "Dockerfile",
            1,
        );
        fn_b.signature = Some("(target, args, context)".to_string());
        graph.add_symbol(fn_b);

        // Must not panic on files without extensions
        let _diagnostics = detect(&graph, Path::new(""));
    }

    // =========================================================================
    // T29: JS cross-file arity mismatch fires for same-language JS
    // =========================================================================

    #[test]
    fn test_contract_mismatch_js_cross_file_arity_fires() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "fetchData",
            SymbolKind::Function,
            Visibility::Public,
            "api_client.js",
            1,
        );
        fn_a.signature = Some("(url)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "fetchData",
            SymbolKind::Function,
            Visibility::Public,
            "api_helper.js",
            1,
        );
        fn_b.signature = Some("(url, options, callback)".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("fetchData") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            !phase2_findings.is_empty(),
            "Phase 2 must fire for same-language JS cross-file arity mismatch. \
             fetchData(url) vs fetchData(url, options, callback) is a genuine contract concern."
        );
    }

    // =========================================================================
    // T30: Regression guard — existing T4 fixture still fires
    // =========================================================================

    #[test]
    fn test_contract_mismatch_existing_t4_fixture_still_fires() {
        let graph = build_contract_mismatch_graph();
        let diagnostics = detect(&graph, Path::new(""));

        let arity_diag = diagnostics
            .iter()
            .find(|d| d.entity.contains("process_data") && d.confidence == Confidence::Moderate);

        assert!(
            arity_diag.is_some(),
            "The existing T4 true positive (process_data in module_a.py vs module_b.py) \
             must survive the FP fix. If this fails, the fix is too aggressive. Got: {:?}",
            diagnostics
                .iter()
                .map(|d| (&d.entity, &d.confidence))
                .collect::<Vec<_>>()
        );
    }

    // =========================================================================
    // T31: Edge case — same name, same .rs file, different arity
    // =========================================================================

    #[test]
    fn test_contract_mismatch_rust_same_file_same_name_different_arity() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "handler.rs",
            10,
        );
        fn_a.signature = Some("(data: &str)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "handler.rs",
            50,
        );
        fn_b.signature = Some("(data: &str, config: &Config)".to_string());
        graph.add_symbol(fn_b);

        // Same-file Rust functions with different arity — should not crash.
        // The Rust exclusion only skips cross-file groups, so same-file still compares.
        let _diagnostics = detect(&graph, Path::new(""));
    }

    // =========================================================================
    // T32: Severity check — Phase 2 severity appropriate for heuristic
    // =========================================================================

    #[test]
    fn test_contract_mismatch_phase2_severity_not_overcalibrated() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            "server_a.py",
            1,
        );
        fn_a.signature = Some("(req)".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            "server_b.py",
            1,
        );
        fn_b.signature = Some("(req, res)".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("handle_request") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            !phase2_findings.is_empty(),
            "Setup: Phase 2 must fire for this scenario"
        );

        for finding in &phase2_findings {
            assert_eq!(
                finding.confidence,
                Confidence::Moderate,
                "Phase 2 findings must be MODERATE confidence (heuristic, may be intentional). Got: {:?}",
                finding.confidence
            );
        }
    }

    // =========================================================================
    // T33: Coverage — empty signature string silently skipped
    // =========================================================================

    #[test]
    fn test_contract_mismatch_empty_signature_string_skipped() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        );
        fn_a.signature = Some(String::new());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        );
        fn_b.signature = Some("(x, y)".to_string());
        graph.add_symbol(fn_b);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("process") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Symbol with empty signature string must be silently excluded from Phase 2 grouping"
        );
    }

    // =========================================================================
    // T34: Coverage — signature without parentheses returns 0 params
    // =========================================================================

    #[test]
    fn test_contract_mismatch_no_parens_signature() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol(
            "getter",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        );
        fn_a.signature = Some("property_name".to_string());
        graph.add_symbol(fn_a);

        let mut fn_b = make_symbol(
            "getter",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        );
        fn_b.signature = Some("(self, key)".to_string());
        graph.add_symbol(fn_b);

        // Should not panic
        let _diagnostics = detect(&graph, Path::new(""));
    }

    // =========================================================================
    // T35: Coverage — whitespace-only signature content
    // =========================================================================

    #[test]
    fn test_contract_mismatch_whitespace_only_signature() {
        let mut graph = Graph::new();

        let mut fn_a = make_symbol("noop", SymbolKind::Function, Visibility::Public, "a.py", 1);
        fn_a.signature = Some("   ".to_string());
        graph.add_symbol(fn_a);

        let diagnostics = detect(&graph, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .all(|d| d.entity != "noop" || d.confidence != Confidence::Moderate),
            "Whitespace-only signature should not produce Phase 2 findings by itself"
        );
    }

    // =========================================================================
    // T36: Coverage — import-annotated symbols excluded from Phase 2
    // =========================================================================

    #[test]
    fn test_contract_mismatch_import_symbols_excluded() {
        let mut graph = Graph::new();

        let mut import_a = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "a.py",
            1,
        );
        import_a.signature = Some("(data)".to_string());
        import_a.annotations.push("import".to_string());
        graph.add_symbol(import_a);

        let mut real_fn = make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "b.py",
            1,
        );
        real_fn.signature = Some("(data, config, extra)".to_string());
        graph.add_symbol(real_fn);

        let diagnostics = detect(&graph, Path::new(""));
        let phase2_findings: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.entity.contains("process") && d.confidence == Confidence::Moderate)
            .collect();

        assert!(
            phase2_findings.is_empty(),
            "Import-annotated symbols must be excluded from Phase 2 grouping. \
             Only one non-import 'process' exists → group size < 2 → no finding."
        );
    }

    // =========================================================================
    // language_from_path unit tests
    // =========================================================================

    #[test]
    fn test_language_from_path_python() {
        assert_eq!(language_from_path(Path::new("module.py")), "python");
    }

    #[test]
    fn test_language_from_path_rust() {
        assert_eq!(language_from_path(Path::new("module.rs")), "rust");
    }

    #[test]
    fn test_language_from_path_javascript_variants() {
        assert_eq!(language_from_path(Path::new("app.js")), "javascript");
        assert_eq!(language_from_path(Path::new("app.jsx")), "javascript");
        assert_eq!(language_from_path(Path::new("app.ts")), "javascript");
        assert_eq!(language_from_path(Path::new("app.tsx")), "javascript");
        assert_eq!(language_from_path(Path::new("app.mjs")), "javascript");
        assert_eq!(language_from_path(Path::new("app.cjs")), "javascript");
    }

    #[test]
    fn test_language_from_path_unknown() {
        assert_eq!(language_from_path(Path::new("file.xyz")), "unknown");
        assert_eq!(language_from_path(Path::new("Makefile")), "unknown");
        assert_eq!(language_from_path(Path::new("")), "unknown");
    }
}
