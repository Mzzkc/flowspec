//! Extracted CLI command logic — testable library functions.
//!
//! These functions contain the core logic previously embedded in the CLI binary's
//! `main.rs`. By living in the library crate, they can be unit-tested directly,
//! improving coverage and catching regressions without integration test overhead.

use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::analyzer::flow::{trace_flows_from, trace_flows_to, FlowPath};
use crate::error::{FlowspecError, ManifestError};
use crate::manifest::types::FlowEntry;
use crate::manifest::{validate_manifest_size, OutputFormatter};
use crate::parser::ir::SymbolId;
use crate::{Config, JsonFormatter, OutputFormat, SarifFormatter, SummaryFormatter, YamlFormatter};

/// Valid diagnostic pattern names for `--checks` validation.
const VALID_PATTERNS: &[&str] = &[
    "isolated_cluster",
    "data_dead_end",
    "phantom_dependency",
    "orphaned_impl",
    "circular_dependency",
    "missing_reexport",
    "contract_mismatch",
    "stale_reference",
    "layer_violation",
    "duplication",
    "partial_wiring",
    "asymmetric_handling",
    "incomplete_migration",
];

/// Trace direction for flow tracing commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDirection {
    /// Trace callees (forward data flow).
    Forward,
    /// Trace callers (backward data flow).
    Backward,
    /// Trace both directions.
    Both,
}

/// Run the `analyze` command: parse, build graph, produce manifest.
///
/// Returns the exit code on success (0 = clean, 2 = critical diagnostics found).
/// Optional filter parameters control which diagnostics appear in output,
/// but exit code is always based on UNFILTERED diagnostics (CI gate contract).
#[allow(clippy::too_many_arguments)]
pub fn run_analyze(
    path: &Path,
    languages: &[String],
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
) -> Result<u8, FlowspecError> {
    if !matches!(
        format,
        OutputFormat::Yaml | OutputFormat::Json | OutputFormat::Sarif | OutputFormat::Summary
    ) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    // Validate check patterns before doing any work
    validate_check_patterns(checks)?;

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    for lang in languages {
        validate_language(lang)?;
    }

    let normalized = normalize_languages(languages);

    tracing::info!("Analyzing project at {}", canonical.display());

    let mut result = crate::analyze(&canonical, &config, &normalized)?;

    // Exit code is based on UNFILTERED diagnostics — CI gate contract
    let has_critical = result.has_critical;

    // Apply diagnostic filters to output only
    apply_diagnostic_filters(
        &mut result.manifest.diagnostics,
        checks,
        severity,
        confidence,
    );

    // Update metadata and summary counts to reflect filtered diagnostics (#16)
    result.manifest.metadata.diagnostic_count = result.manifest.diagnostics.len() as u64;
    recompute_diagnostic_summary(
        &result.manifest.diagnostics,
        &mut result.manifest.summary.diagnostic_summary,
    );

    let output = format_with(format, |f| f.format_manifest(&result.manifest))?;

    validate_manifest_size(&output, result.source_bytes)?;

    write_output(&output, output_path)?;

    if has_critical {
        Ok(2)
    } else {
        Ok(0)
    }
}

/// Run the `diagnose` command: run diagnostics with optional filters.
///
/// Returns the exit code on success (0 = clean, 2 = findings).
#[allow(clippy::too_many_arguments)]
pub fn run_diagnose(
    path: &Path,
    languages: &[String],
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8, FlowspecError> {
    if !matches!(
        format,
        OutputFormat::Yaml | OutputFormat::Json | OutputFormat::Sarif | OutputFormat::Summary
    ) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    for lang in languages {
        validate_language(lang)?;
    }

    let checks_filter = if checks.is_empty() {
        None
    } else {
        Some(checks)
    };

    let normalized = normalize_languages(languages);

    tracing::info!("Running diagnostics on {}", canonical.display());

    let (diagnostics, has_findings) = crate::diagnose(
        &canonical,
        &config,
        &normalized,
        severity,
        confidence,
        checks_filter,
    )?;

    let output = format_with(format, |f| f.format_diagnostics(&diagnostics))?;

    write_output(&output, output_path)?;

    if has_findings {
        Ok(2)
    } else {
        Ok(0)
    }
}

/// Run the `trace` command: trace a single symbol's flow through the codebase.
///
/// Uses graph-direct tracing via `trace_flows_from()` (forward) and
/// `trace_flows_to()` (backward), bypassing the manifest.flows pre-computed
/// entry-point-based flows. This fixes the 3-cycle carry bug where symbols
/// with known call edges but no reachable `main()` returned empty results.
///
/// Returns the exit code on success.
#[allow(clippy::too_many_arguments)]
pub fn run_trace(
    path: &Path,
    symbol: &str,
    languages: &[String],
    depth: usize,
    direction: TraceDirection,
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8, FlowspecError> {
    // Guard unsupported formats
    if !matches!(
        format,
        OutputFormat::Yaml | OutputFormat::Json | OutputFormat::Sarif | OutputFormat::Summary
    ) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;

    for lang in languages {
        validate_language(lang)?;
    }
    let normalized_languages = normalize_languages(languages);

    tracing::info!("Tracing symbol '{}' in {}", symbol, canonical.display());

    let result = crate::analyze(&canonical, &config, &normalized_languages)?;

    // Symbol matching: exact qualified name → name part → substring
    let matched_entity = find_matching_symbol(symbol, &result.manifest.entities)?;

    // Look up the matched symbol's SymbolId in the graph
    let matched_sym_id = find_symbol_id_in_graph(&result.graph, &matched_entity)?;

    // Graph-direct tracing — FROM semantics (forward) / TO semantics (backward)
    let flow_paths = match direction {
        TraceDirection::Forward => trace_flows_from(&result.graph, matched_sym_id, depth),
        TraceDirection::Backward => trace_flows_to(&result.graph, matched_sym_id, depth),
        TraceDirection::Both => {
            let mut forward = trace_flows_from(&result.graph, matched_sym_id, depth);
            let backward = trace_flows_to(&result.graph, matched_sym_id, depth);
            forward.extend(backward);
            forward
        }
    };

    // Convert FlowPath (graph-level) → FlowEntry (manifest-level)
    let mut flow_entries = flow_paths_to_entries(&result.graph, &flow_paths, &matched_entity);

    // Deduplicate flows for --direction both (forward + backward may overlap)
    if direction == TraceDirection::Both {
        flow_entries = deduplicate_flows(flow_entries);
    }

    // Apply depth truncation to steps
    for flow in &mut flow_entries {
        if flow.steps.len() > depth {
            flow.steps.truncate(depth);
        }
    }

    // Serialize focused output directly (bypasses OutputFormatter — no manifest wrapper)
    let output = match format {
        OutputFormat::Yaml => {
            serde_yaml::to_string(&flow_entries).map_err(|e| FlowspecError::Manifest {
                reason: format!("YAML serialization failed: {}", e),
            })?
        }
        OutputFormat::Json => {
            serde_json::to_string_pretty(&flow_entries).map_err(|e| FlowspecError::Manifest {
                reason: format!("JSON serialization failed: {}", e),
            })?
        }
        OutputFormat::Sarif => format_trace_sarif(&flow_entries)?,
        OutputFormat::Summary => {
            let mut lines = Vec::new();
            lines.push(format!(
                "Trace: {} ({} flow(s) matched)",
                matched_entity,
                flow_entries.len()
            ));
            lines.push(String::new());
            for flow in &flow_entries {
                lines.push(format!("  {} -> {}", flow.entry, flow.exit));
                for step in &flow.steps {
                    lines.push(format!("    {} ({})", step.entity, step.action));
                }
            }
            lines.join("\n")
        }
    };

    write_output(&output, output_path)?;

    Ok(0)
}

/// Find a SymbolId in the graph by matching qualified_name.
///
/// Iterates all symbols in the graph and returns the first match on qualified_name.
/// Returns SymbolNotFound if no symbol with the given qualified name exists.
pub fn find_symbol_id_in_graph(
    graph: &crate::Graph,
    qualified_name: &str,
) -> Result<SymbolId, FlowspecError> {
    for (sym_id, sym) in graph.all_symbols() {
        if sym.qualified_name == qualified_name {
            return Ok(sym_id);
        }
    }
    Err(FlowspecError::SymbolNotFound(format!(
        "Symbol '{}' not found in graph. Run `flowspec analyze` to see available entities.",
        qualified_name
    )))
}

/// Convert graph-level FlowPaths to manifest-level FlowEntries.
///
/// Maps SymbolIds to qualified names via graph lookup, matching the conversion
/// logic in lib.rs for consistency.
pub fn flow_paths_to_entries(
    graph: &crate::Graph,
    paths: &[FlowPath],
    matched_entity: &str,
) -> Vec<FlowEntry> {
    paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let entry_name = graph
                .get_symbol(path.entry)
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| matched_entity.to_string());

            let last_step = path.steps.last();
            let exit_name = last_step
                .and_then(|step| graph.get_symbol(step.symbol))
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| entry_name.clone());

            let steps: Vec<crate::manifest::types::FlowStep> = path
                .steps
                .iter()
                .map(|step| {
                    let entity = graph
                        .get_symbol(step.symbol)
                        .map(|s| s.qualified_name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    crate::manifest::types::FlowStep {
                        entity,
                        action: "call".to_string(),
                        in_type: "unknown".to_string(),
                        out_type: "unknown".to_string(),
                    }
                })
                .collect();

            let description = if path.is_cyclic {
                format!("Cyclic flow from {}", entry_name)
            } else {
                format!("Flow from {} to {}", entry_name, exit_name)
            };

            FlowEntry {
                id: format!("F{:03}", i + 1),
                description,
                entry: entry_name,
                exit: exit_name,
                steps,
                issues: Vec::new(),
            }
        })
        .collect()
}

/// Deduplicate flow entries, preserving first occurrence and re-numbering IDs.
///
/// Two flows are considered duplicates if they share the same entry, exit,
/// and step entity sequence. The dedup key includes steps (not just entry/exit)
/// to preserve flows with the same endpoints but different intermediate paths.
pub fn deduplicate_flows(flows: Vec<FlowEntry>) -> Vec<FlowEntry> {
    let mut seen = HashSet::new();
    let mut unique: Vec<FlowEntry> = Vec::with_capacity(flows.len());

    for flow in flows {
        let step_entities: String = flow
            .steps
            .iter()
            .map(|s| s.entity.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let key = format!("{}|{}|{}", flow.entry, flow.exit, step_entities);

        if seen.insert(key) {
            unique.push(flow);
        }
    }

    // Re-number IDs sequentially after dedup
    for (i, flow) in unique.iter_mut().enumerate() {
        flow.id = format!("F{:03}", i + 1);
    }

    unique
}

/// Validate that all `--checks` pattern names are valid.
///
/// Returns an error listing valid pattern names if any invalid name is found.
pub fn validate_check_patterns(checks: &[String]) -> Result<(), FlowspecError> {
    for pattern in checks {
        if !pattern.is_empty() && !VALID_PATTERNS.contains(&pattern.as_str()) {
            return Err(FlowspecError::UnknownPattern {
                pattern: pattern.clone(),
            });
        }
    }
    Ok(())
}

/// Recompute diagnostic summary counts from the actual diagnostics list.
///
/// After filtering, the pre-computed `DiagnosticSummary` (critical/warning/info)
/// becomes stale. This function recalculates from the filtered diagnostics.
pub fn recompute_diagnostic_summary(
    diagnostics: &[crate::manifest::types::DiagnosticEntry],
    summary: &mut crate::manifest::types::DiagnosticSummary,
) {
    summary.critical = diagnostics
        .iter()
        .filter(|d| d.severity == "critical")
        .count() as u64;
    summary.warning = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count() as u64;
    summary.info = diagnostics.iter().filter(|d| d.severity == "info").count() as u64;
    summary.top_issues = diagnostics
        .iter()
        .take(5)
        .map(|d| format!("{}: {}", d.pattern, d.message))
        .collect();
}

/// Apply diagnostic filters to a diagnostics list in-place.
///
/// Filters by severity (>=), confidence (>=), and pattern name.
pub fn apply_diagnostic_filters(
    diagnostics: &mut Vec<crate::manifest::types::DiagnosticEntry>,
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
) {
    if let Some(min_severity) = severity {
        diagnostics.retain(|d| {
            let sev =
                crate::Severity::from_str_checked(&d.severity).unwrap_or(crate::Severity::Info);
            sev >= min_severity
        });
    }

    if let Some(min_confidence) = confidence {
        diagnostics.retain(|d| {
            let conf = crate::Confidence::from_str_checked(&d.confidence)
                .unwrap_or(crate::Confidence::Low);
            conf >= min_confidence
        });
    }

    let non_empty_checks: Vec<&String> = checks.iter().filter(|c| !c.is_empty()).collect();
    if !non_empty_checks.is_empty() {
        diagnostics.retain(|d| non_empty_checks.iter().any(|c| d.pattern == c.as_str()));
    }
}

/// Find a matching symbol in the entity list using cascading match strategy.
///
/// Priority: exact qualified name → exact name part → substring match.
/// Returns an error if no match or multiple ambiguous matches are found.
pub fn find_matching_symbol(
    symbol: &str,
    entities: &[crate::manifest::types::EntityEntry],
) -> Result<String, FlowspecError> {
    // 1. Exact match on qualified name (e.g., "main.py::process")
    let exact_matches: Vec<&crate::manifest::types::EntityEntry> =
        entities.iter().filter(|e| e.id == symbol).collect();
    if exact_matches.len() == 1 {
        return Ok(exact_matches[0].id.clone());
    }

    // 2. Exact match on the name part (after last ::)
    let name_matches: Vec<&crate::manifest::types::EntityEntry> = entities
        .iter()
        .filter(|e| {
            e.id.rsplit("::")
                .next()
                .map(|name| name == symbol)
                .unwrap_or(false)
        })
        .collect();
    if name_matches.len() == 1 {
        return Ok(name_matches[0].id.clone());
    }
    if name_matches.len() > 1 {
        let options: Vec<String> = name_matches
            .iter()
            .map(|e| format!("{} ({})", e.id, e.loc))
            .collect();
        return Err(FlowspecError::SymbolNotFound(format!(
            "Symbol '{}' matches multiple entities: {}. Use a qualified name to disambiguate.",
            symbol,
            options.join(", ")
        )));
    }

    // 3. Substring match
    let substring_matches: Vec<&crate::manifest::types::EntityEntry> =
        entities.iter().filter(|e| e.id.contains(symbol)).collect();
    if substring_matches.len() == 1 {
        return Ok(substring_matches[0].id.clone());
    }
    if substring_matches.len() > 1 {
        let options: Vec<String> = substring_matches
            .iter()
            .map(|e| format!("{} ({})", e.id, e.loc))
            .collect();
        return Err(FlowspecError::SymbolNotFound(format!(
            "Symbol '{}' matches multiple entities: {}. Use a qualified name to disambiguate.",
            symbol,
            options.join(", ")
        )));
    }

    // No match
    Err(FlowspecError::SymbolNotFound(format!(
        "Symbol '{}' not found. Run `flowspec analyze` to see available entities.",
        symbol
    )))
}

/// Format flow entries as a minimal SARIF v2.1.0 envelope.
pub fn format_trace_sarif(flow_entries: &[FlowEntry]) -> Result<String, FlowspecError> {
    let results: Vec<serde_json::Value> = flow_entries
        .iter()
        .map(|flow| {
            serde_json::json!({
                "ruleId": "flow-trace",
                "message": { "text": flow.description },
                "properties": {
                    "entry": flow.entry,
                    "exit": flow.exit,
                    "steps": flow.steps.iter().map(|s| {
                        serde_json::json!({
                            "entity": s.entity,
                            "action": s.action,
                        })
                    }).collect::<Vec<_>>(),
                }
            })
        })
        .collect();

    let sarif = serde_json::json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "flowspec",
                    "version": env!("CARGO_PKG_VERSION"),
                    "informationUri": "https://github.com/anthropics/flowspec"
                }
            },
            "results": results,
        }]
    });

    serde_json::to_string_pretty(&sarif).map_err(|e| FlowspecError::Manifest {
        reason: format!("SARIF serialization failed: {}", e),
    })
}

/// Resolve a path, checking existence.
pub fn resolve_path(path: &Path) -> Result<PathBuf, FlowspecError> {
    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }

    let canonical = if path.is_relative() {
        std::env::current_dir()
            .map_err(|e| FlowspecError::Io {
                path: path.to_path_buf(),
                source: e,
            })?
            .join(path)
    } else {
        path.to_path_buf()
    };

    if !canonical.exists() {
        return Err(FlowspecError::TargetNotFound {
            path: path.to_path_buf(),
        });
    }

    Ok(canonical)
}

/// Write output to stdout or a file.
pub fn write_output(content: &str, output_path: Option<&Path>) -> Result<(), FlowspecError> {
    if let Some(path) = output_path {
        std::fs::write(path, content).map_err(|e| FlowspecError::Io {
            path: path.to_path_buf(),
            source: e,
        })?;
    } else {
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        handle
            .write_all(content.as_bytes())
            .map_err(|e| FlowspecError::Io {
                path: PathBuf::from("<stdout>"),
                source: e,
            })?;
    }
    Ok(())
}

/// Dispatch formatting to the correct formatter based on the selected format.
pub fn format_with<F>(format: OutputFormat, f: F) -> Result<String, FlowspecError>
where
    F: FnOnce(&dyn OutputFormatter) -> Result<String, ManifestError>,
{
    let result = match format {
        OutputFormat::Yaml => f(&YamlFormatter::new()),
        OutputFormat::Json => f(&JsonFormatter::new()),
        OutputFormat::Sarif => f(&SarifFormatter::new()),
        OutputFormat::Summary => f(&SummaryFormatter::new()),
    };
    result.map_err(FlowspecError::from)
}

/// Get the display name for a format.
pub fn format_name(format: OutputFormat) -> &'static str {
    match format {
        OutputFormat::Yaml => "yaml",
        OutputFormat::Json => "json",
        OutputFormat::Sarif => "sarif",
        OutputFormat::Summary => "summary",
    }
}

/// Normalize a language alias to its canonical name.
///
/// Accepts common abbreviations: "ts" → "typescript", "js" → "javascript",
/// "py" → "python". Canonical names pass through unchanged.
pub fn normalize_language(lang: &str) -> String {
    match lang {
        "ts" => "typescript".to_string(),
        "js" => "javascript".to_string(),
        "py" => "python".to_string(),
        other => other.to_string(),
    }
}

/// Validate a language name against v1 supported languages.
///
/// Accepts both canonical names and common abbreviations (e.g., "ts" for "typescript").
pub fn validate_language(lang: &str) -> Result<(), FlowspecError> {
    let normalized = normalize_language(lang);
    match normalized.as_str() {
        "python" | "javascript" | "typescript" | "rust" => Ok(()),
        _ => Err(FlowspecError::UnsupportedLanguage {
            language: lang.to_string(),
        }),
    }
}

/// Normalize a list of language arguments, expanding aliases.
pub fn normalize_languages(languages: &[String]) -> Vec<String> {
    languages.iter().map(|l| normalize_language(l)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::types::{EntityEntry, Manifest};

    fn entity_with_id(id: &str) -> EntityEntry {
        EntityEntry {
            id: id.to_string(),
            kind: "fn".to_string(),
            vis: "pub".to_string(),
            sig: String::new(),
            loc: "test.py:1".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        }
    }

    // T5: normalize_language handles aliases
    #[test]
    fn normalize_language_handles_aliases() {
        assert_eq!(normalize_language("ts"), "typescript");
        assert_eq!(normalize_language("js"), "javascript");
        assert_eq!(normalize_language("py"), "python");
        assert_eq!(normalize_language("rust"), "rust");
        assert_eq!(normalize_language("python"), "python");
    }

    // T6: normalize_language passes unknown languages through
    #[test]
    fn normalize_language_passes_unknown_through() {
        assert_eq!(normalize_language("go"), "go");
        assert_eq!(normalize_language(""), "");
        assert_eq!(normalize_language("PYTHON"), "PYTHON"); // case-sensitive
    }

    // T7: validate_language rejects unsupported languages
    #[test]
    fn validate_language_rejects_unsupported() {
        assert!(validate_language("go").is_err());
        assert!(validate_language("java").is_err());
        assert!(validate_language("").is_err());
        assert!(validate_language("PYTHON").is_err()); // case-sensitive
    }

    #[test]
    fn validate_language_accepts_supported_and_aliases() {
        assert!(validate_language("python").is_ok());
        assert!(validate_language("javascript").is_ok());
        assert!(validate_language("typescript").is_ok());
        assert!(validate_language("rust").is_ok());
        assert!(validate_language("ts").is_ok());
        assert!(validate_language("js").is_ok());
        assert!(validate_language("py").is_ok());
    }

    // T8: find_matching_symbol — exact match takes priority
    #[test]
    fn find_matching_symbol_exact_match_wins() {
        let entities = vec![
            entity_with_id("module::process"),
            entity_with_id("module::process_data"),
        ];
        let result = find_matching_symbol("module::process", &entities);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "module::process");
    }

    // T9: find_matching_symbol — ambiguous substring match errors with candidates
    #[test]
    fn find_matching_symbol_ambiguous_lists_candidates() {
        let entities = vec![
            entity_with_id("module::process_a"),
            entity_with_id("module::process_b"),
        ];
        let result = find_matching_symbol("process", &entities);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("process_a"),
            "Must list candidate: process_a.\n{}",
            err_msg
        );
        assert!(
            err_msg.contains("process_b"),
            "Must list candidate: process_b.\n{}",
            err_msg
        );
    }

    // T10: find_matching_symbol — no match returns SymbolNotFound
    #[test]
    fn find_matching_symbol_no_match_returns_error() {
        let entities = vec![entity_with_id("module::handler")];
        let result = find_matching_symbol("nonexistent", &entities);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("not found") || err_msg.contains("symbol"),
            "No-match error should mention 'not found'.\n{}",
            err_msg
        );
    }

    // T11: find_matching_symbol — empty entities list
    #[test]
    fn find_matching_symbol_empty_entities() {
        let entities: Vec<EntityEntry> = vec![];
        let result = find_matching_symbol("anything", &entities);
        assert!(
            result.is_err(),
            "Empty entity list must return error, not panic"
        );
    }

    // T12: format_name returns correct display names
    #[test]
    fn format_name_returns_expected_strings() {
        assert_eq!(format_name(OutputFormat::Yaml), "yaml");
        assert_eq!(format_name(OutputFormat::Json), "json");
        assert_eq!(format_name(OutputFormat::Sarif), "sarif");
        assert_eq!(format_name(OutputFormat::Summary), "summary");
    }

    // T13: normalize_languages batch + empty
    #[test]
    fn normalize_languages_batch() {
        let input = vec!["ts".to_string(), "py".to_string(), "rust".to_string()];
        let result = normalize_languages(&input);
        assert_eq!(result, vec!["typescript", "python", "rust"]);
    }

    #[test]
    fn normalize_languages_empty_input() {
        let input: Vec<String> = vec![];
        let result = normalize_languages(&input);
        assert!(result.is_empty());
    }

    // format_with dispatches correctly for all formats
    #[test]
    fn format_with_dispatches_yaml() {
        let manifest = Manifest::empty();
        let result = format_with(OutputFormat::Yaml, |f| f.format_manifest(&manifest));
        assert!(result.is_ok());
    }

    #[test]
    fn format_with_dispatches_json() {
        let manifest = Manifest::empty();
        let result = format_with(OutputFormat::Json, |f| f.format_manifest(&manifest));
        assert!(result.is_ok());
    }

    #[test]
    fn format_with_dispatches_sarif() {
        let manifest = Manifest::empty();
        let result = format_with(OutputFormat::Sarif, |f| f.format_manifest(&manifest));
        assert!(result.is_ok());
    }

    #[test]
    fn format_with_dispatches_summary() {
        let manifest = Manifest::empty();
        let result = format_with(OutputFormat::Summary, |f| f.format_manifest(&manifest));
        assert!(
            result.is_ok(),
            "Summary format must work: {:?}",
            result.err()
        );
    }

    // resolve_path rejects empty path
    #[test]
    fn resolve_path_rejects_empty() {
        let result = resolve_path(Path::new(""));
        assert!(result.is_err());
    }

    // resolve_path rejects nonexistent path
    #[test]
    fn resolve_path_rejects_nonexistent() {
        let result = resolve_path(Path::new("/nonexistent/path/that/should/not/exist"));
        assert!(result.is_err());
    }

    // write_output to temp file
    #[test]
    fn write_output_to_file() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("test_output.txt");
        write_output("hello world", Some(&file)).unwrap();
        let content = std::fs::read_to_string(&file).unwrap();
        assert_eq!(content, "hello world");
    }

    // validate_check_patterns
    #[test]
    fn validate_check_patterns_accepts_valid() {
        assert!(validate_check_patterns(&["data_dead_end".to_string()]).is_ok());
        assert!(validate_check_patterns(&["phantom_dependency".to_string()]).is_ok());
        assert!(validate_check_patterns(&[]).is_ok());
    }

    #[test]
    fn validate_check_patterns_rejects_invalid() {
        let result = validate_check_patterns(&["nonexistent_pattern".to_string()]);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("nonexistent_pattern"));
    }
}
