//! Extracted CLI command logic — testable library functions.
//!
//! These functions contain the core logic previously embedded in the CLI binary's
//! `main.rs`. By living in the library crate, they can be unit-tested directly,
//! improving coverage and catching regressions without integration test overhead.

pub use crate::report_records::expand_report_records;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::analyzer::flow::FlowPath;
use crate::error::{FlowspecError, ManifestError};
use crate::manifest::types::{DiagnosticEntry, FlowEntry, Manifest};
use crate::manifest::{validate_manifest_size, OutputFormatter};
use crate::parser::ir::SymbolId;
use crate::{Config, JsonFormatter, OutputFormat, SarifFormatter, SummaryFormatter, YamlFormatter};

/// Valid diagnostic pattern names for `--checks` validation.
const VALID_PATTERNS: &[&str] = &crate::registry::PATTERN_IDS;

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
    run_analyze_with_gate(
        path,
        languages,
        format,
        output_path,
        config_path,
        checks,
        severity,
        confidence,
        None,
        None,
    )
}

/// Analyze with a gate threshold independent of presentation filters.
#[allow(clippy::too_many_arguments)]
pub fn run_analyze_with_gate(
    path: &Path,
    languages: &[String],
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
    gate_severity: Option<crate::Severity>,
    gate_confidence: Option<crate::Confidence>,
) -> Result<u8, FlowspecError> {
    run_analyze_with_gate_and_mode(
        path,
        languages,
        format,
        output_path,
        config_path,
        checks,
        severity,
        confidence,
        gate_severity,
        gate_confidence,
        crate::cache::AnalysisMode::Full,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_analyze_with_gate_and_mode(
    path: &Path,
    languages: &[String],
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
    gate_severity: Option<crate::Severity>,
    gate_confidence: Option<crate::Confidence>,
    mode: crate::cache::AnalysisMode,
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

    let result = prepare_report_with_analysis_options(
        &canonical,
        &config,
        &normalized,
        &crate::ReportOptions {
            checks: checks.to_vec(),
            severity,
            confidence,
            gate_severity: gate_severity.unwrap_or(crate::Severity::Critical),
            gate_confidence: gate_confidence.unwrap_or(crate::Confidence::Low),
        },
        &crate::AnalysisOptions {
            mode,
            ..Default::default()
        },
    )?;
    if !result.cache_stats.fallbacks.is_empty() {
        tracing::warn!("Cache fallback: {:?}", result.cache_stats.fallbacks);
    }

    let output = render_report(&result.manifest, format)?;

    // Check exactly the bytes that will be written. The compact projection
    // still carries the full finding catalog and input inventory in authority.
    validate_manifest_size(&output, result.source_bytes, format_name(format))?;

    write_output(&output, output_path)?;

    Ok(crate::report_exit_code(&result.manifest, None))
}

/// Prepare one authoritative report for every shipped consumer.
pub(crate) fn prepare_report(
    path: &Path,
    config: &Config,
    languages: &[String],
    options: &crate::ReportOptions,
) -> Result<crate::AnalysisResult, FlowspecError> {
    prepare_report_with_analysis_options(
        path,
        config,
        languages,
        options,
        &crate::AnalysisOptions::default(),
    )
}

pub(crate) fn prepare_report_with_analysis_options(
    path: &Path,
    config: &Config,
    languages: &[String],
    options: &crate::ReportOptions,
    analysis: &crate::AnalysisOptions,
) -> Result<crate::AnalysisResult, FlowspecError> {
    let checks = &options.checks;
    let severity = options.severity;
    let confidence = options.confidence;
    validate_check_patterns(checks)?;
    let mut result = crate::analyze_with_options(path, config, languages, analysis)?;

    // Exit code is based on UNFILTERED diagnostics — CI gate contract
    if let Some(authority) = result.manifest.authority.as_mut() {
        crate::report_authority::set_gate(
            authority,
            &options.gate_severity.to_string(),
            &options.gate_confidence.to_string(),
        );
    }

    // Apply diagnostic filters to output only
    apply_diagnostic_filters(
        &mut result.manifest.diagnostics,
        checks,
        severity,
        confidence,
    );
    result.manifest.displayed_findings_total = result.manifest.diagnostics.len() as u64;
    if let Some(authority) = result.manifest.authority.as_mut() {
        authority["display"]["filters"] = serde_json::json!({
            "checks":checks,"min_severity":severity.map(|s|s.to_string()),
            "min_confidence":confidence.map(|c|c.to_string())});
        let indices: Vec<usize> = authority["findings"]["all"]
            .as_array()
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, finding)| {
                let pattern = finding["pattern_id"].as_str().unwrap_or("");
                (checks.is_empty() || checks.iter().any(|c| c == pattern))
                    && severity.is_none_or(|min| {
                        crate::Severity::from_str_checked(
                            finding["severity"].as_str().unwrap_or(""),
                        )
                        .is_some_and(|s| s >= min)
                    })
                    && confidence.is_none_or(|min| {
                        crate::Confidence::from_str_checked(
                            finding["confidence"].as_str().unwrap_or(""),
                        )
                        .is_some_and(|c| c >= min)
                    })
            })
            .map(|(i, _)| i)
            .collect();
        authority["display"]["indices"] = serde_json::json!(indices);
        authority["continuation"]["shown"] = serde_json::json!(indices.len());
        authority["continuation"]["matched_total"] = serde_json::json!(indices.len());
        authority["continuation"]["full"] = authority["findings"]["total"].clone();
    }

    // Update metadata and summary counts to reflect filtered diagnostics (#16)
    result.manifest.metadata.diagnostic_count = result.manifest.diagnostics.len() as u64;
    recompute_diagnostic_summary(
        &result.manifest.diagnostics,
        &mut result.manifest.summary.diagnostic_summary,
    );

    // Requested consumers validate their actual serialized bytes before writing.
    // A complete catalog that cannot fit a structured output remains complete:
    // its Summary and in-memory report must not acquire serialization loss.
    Ok(result)
}

/// Establish the shared public report form before any caller-specific gate/filter.
pub(crate) fn normalize_report(result: &mut crate::AnalysisResult) -> Result<(), FlowspecError> {
    // Already-normalized reports retain identity and loss accounting on reuse.
    if result.manifest.authority.as_ref().is_some_and(|a| {
        matches!(
            a["analysis_graph"]["encoding"].as_str(),
            Some("flowspec.graph.tables.v1" | "flowspec.graph.tables.v2")
        ) || a["analysis_graph"]["projection"]["analysis_graph"] == "omitted"
    }) {
        result.manifest.sync_pattern_state();
        return Ok(());
    }
    // Decide the available analysis projection before any presentation choice.
    // Evaluate the same full report for both structured formats, so a display
    // filter or format cannot manufacture a different execution or gate state.
    if let Some(authority) = result.manifest.authority.as_mut() {
        let total = authority["findings"]["total"].as_u64().unwrap_or(0);
        authority["display"]["indices"] = serde_json::json!((0..total).collect::<Vec<_>>());
        authority["continuation"]["shown"] = serde_json::json!(total);
        authority["continuation"]["matched_total"] = serde_json::json!(total);
        authority["continuation"]["full"] = serde_json::json!(total);
    }
    let full_json =
        serde_json::to_string(&result.manifest).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        })?;
    let full_yaml =
        serde_yaml::to_string(&result.manifest).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        })?;
    // Share one lossless graph representation across report consumers before
    // display filtering. Complete analysis never acquires serialization loss.
    if full_json.len() as u64 > result.source_bytes.max(1024).saturating_mul(10)
        || full_yaml.len() as u64 > result.source_bytes.max(1024).saturating_mul(10)
    {
        if let Some(authority) = result.manifest.authority.as_mut() {
            if let Some(packed) = graph_tables(&authority["analysis_graph"]) {
                authority["analysis_graph"] = packed;
                result.manifest.losses.clear();
                result.manifest.pattern_state.clear();
                crate::report_authority::refresh_identity(authority);
            }
        }
    }
    // Lossless graph tables are the ordinary normal form. A renderer may refuse
    // an oversized report; normalization never discards its semantic graph.
    result.manifest.sync_pattern_state();
    Ok(())
}

fn render_report(
    manifest: &crate::AnalysisReport,
    format: OutputFormat,
) -> Result<String, FlowspecError> {
    let output = match format {
        OutputFormat::Json => {
            serde_json::to_string(manifest).map_err(|e| FlowspecError::Manifest {
                reason: e.to_string(),
            })?
        }
        OutputFormat::Yaml => analyze_yaml(manifest)?,
        OutputFormat::Summary => {
            let completeness = manifest.completeness.as_ref().expect("analyzed report");
            let mut state = format!(
                "schema_version: {}\nexecution: {}\nfindings_present: {}\noutcome: {}\nselected: {}\nread: {}\nparsed: {}\nrejected: {}\n",
                manifest.schema_version.expect("analyzed report"),
                execution_name(manifest.execution.expect("analyzed report")),
                manifest.findings_present.expect("analyzed report"),
                outcome_name(manifest.outcome.expect("analyzed report")),
                completeness.selected,
                completeness.read,
                completeness.parsed,
                completeness.rejected,
            );
            for file in &completeness.rejected_files {
                state.push_str(&format!("{}: {}\n", file.path, file.reason));
            }
            state.push('\n');
            state.push_str(&SummaryFormatter::new().format_manifest(manifest)?);
            state
        }
        OutputFormat::Sarif => format_with(format, |f| f.format_manifest(manifest))?,
    };

    Ok(output)
}

pub(crate) fn analyze_yaml(manifest: &Manifest) -> Result<String, FlowspecError> {
    let projected = manifest.authority.as_ref().is_some_and(|authority| {
        authority["analysis_graph"]["projection"]["analysis_graph"] == "omitted"
            || matches!(
                authority["analysis_graph"]["encoding"].as_str(),
                Some("flowspec.graph.tables.v1" | "flowspec.graph.tables.v2")
            )
    });
    if !projected {
        return serde_yaml::to_string(manifest).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        });
    }
    // JSON flow values are valid YAML. Keep ordinary top-level YAML sections
    // while avoiding the indentation overhead of the authoritative payload.
    let value = serde_json::to_value(manifest).map_err(|e| FlowspecError::Manifest {
        reason: e.to_string(),
    })?;
    let mut output = String::new();
    let fields = value.as_object().expect("manifest object");
    let legacy_sections = [
        "metadata",
        "summary",
        "diagnostics",
        "entities",
        "flows",
        "boundaries",
        "dependency_graph",
        "type_flows",
    ];
    let keys = fields
        .keys()
        .map(String::as_str)
        .filter(|key| !legacy_sections.contains(key))
        .chain(legacy_sections);
    for key in keys {
        let value = &fields[key];
        let flow = serde_json::to_string(value).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        })?;
        output.push_str(&format!("{key}: {flow}\n"));
    }
    Ok(output)
}

fn execution_name(execution: crate::Execution) -> &'static str {
    match execution {
        crate::Execution::Complete => "complete",
        crate::Execution::Incomplete => "incomplete",
        crate::Execution::Error => "error",
    }
}

fn outcome_name(outcome: crate::Outcome) -> &'static str {
    match outcome {
        crate::Outcome::Clean => "clean",
        crate::Outcome::Findings => "findings",
        crate::Outcome::Incomplete => "incomplete",
        crate::Outcome::Error => "error",
    }
}

/// Run the `diagnose` command: run diagnostics with optional filters.
///
/// Returns 0 for a complete passing gate, 2 for a complete failing gate,
/// and 3 for incomplete execution, independent of presentation filters.
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
    run_diagnose_with_gate(
        path,
        languages,
        checks,
        severity,
        confidence,
        format,
        output_path,
        config_path,
        None,
        None,
    )
}

/// Diagnose with independent gate thresholds; default is any qualified finding.
#[allow(clippy::too_many_arguments)]
pub fn run_diagnose_with_gate(
    path: &Path,
    languages: &[String],
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
    gate_severity: Option<crate::Severity>,
    gate_confidence: Option<crate::Confidence>,
) -> Result<u8, FlowspecError> {
    run_diagnose_with_gate_and_mode(
        path,
        languages,
        checks,
        severity,
        confidence,
        format,
        output_path,
        config_path,
        gate_severity,
        gate_confidence,
        crate::cache::AnalysisMode::Full,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn run_diagnose_with_gate_and_mode(
    path: &Path,
    languages: &[String],
    checks: &[String],
    severity: Option<crate::Severity>,
    confidence: Option<crate::Confidence>,
    format: OutputFormat,
    output_path: Option<&Path>,
    config_path: Option<&Path>,
    gate_severity: Option<crate::Severity>,
    gate_confidence: Option<crate::Confidence>,
    mode: crate::cache::AnalysisMode,
) -> Result<u8, FlowspecError> {
    if path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }
    validate_check_patterns(checks)?;
    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;
    for language in languages {
        validate_language(language)?;
    }
    let result = prepare_report_with_analysis_options(
        &canonical,
        &config,
        &normalize_languages(languages),
        &crate::ReportOptions {
            checks: checks.to_vec(),
            severity,
            confidence,
            gate_severity: gate_severity.unwrap_or(crate::Severity::Info),
            gate_confidence: gate_confidence.unwrap_or(crate::Confidence::Low),
        },
        &crate::AnalysisOptions {
            mode,
            ..Default::default()
        },
    )?;
    if !result.cache_stats.fallbacks.is_empty() {
        tracing::warn!("Cache fallback: {:?}", result.cache_stats.fallbacks);
    }

    let output = render_report(&result.manifest, format)?;
    validate_manifest_size(&output, result.source_bytes, format_name(format))?;
    write_output(&output, output_path)?;
    Ok(crate::report_exit_code(&result.manifest, None))
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

    let forward = crate::analyzer::flow::trace_report(&result.graph, matched_sym_id, depth, false);
    let backward = crate::analyzer::flow::trace_report(&result.graph, matched_sym_id, depth, true);
    let (flow_paths, incomplete) = match direction {
        TraceDirection::Forward => (forward.paths, forward.incomplete),
        TraceDirection::Backward => (backward.paths, backward.incomplete),
        TraceDirection::Both => {
            let mut paths = forward.paths;
            paths.extend(backward.paths);
            (paths, forward.incomplete || backward.incomplete)
        }
    };
    // Convert FlowPath (graph-level) → FlowEntry (manifest-level)
    let mut flow_entries = flow_paths_to_entries(&result.graph, &flow_paths, &matched_entity);

    // The source query already supplies actual invocations. Equal binding
    // sequences can belong to distinct call sites and must remain visible.

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

    Ok(if incomplete { 3 } else { 0 })
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
                        action: match step.edge_kind {
                            crate::parser::ir::EdgeKind::Calls => "call".into(),
                            kind => format!("{:?}", kind).to_lowercase(),
                        },
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
                issues: if path.truncated {
                    vec!["depth_truncation".into()]
                } else {
                    Vec::new()
                },
            }
        })
        .collect()
}

/// Valid manifest section names for `--section` validation in `diff`.
// TODO: Add "metadata", "summary", "flows", "boundaries", "dependency_graph", "type_flows"
// when compute_diff() gains support for these sections.
const VALID_SECTIONS: &[&str] = &["entities", "diagnostics"];

/// Result of comparing two manifests.
///
/// Captures structural differences across entities, diagnostics, and metadata.
/// Serializable for output in any supported format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiffResult {
    /// Entity IDs present in new but not old.
    pub entities_added: Vec<String>,
    /// Entity IDs present in old but not new.
    pub entities_removed: Vec<String>,
    /// Entity IDs present in both but with field changes.
    pub entities_changed: Vec<EntityChange>,
    /// Diagnostics in new but not old (matched by pattern+entity+loc).
    pub diagnostics_new: Vec<DiagnosticIdentity>,
    /// Diagnostics in old but not new (matched by pattern+entity+loc).
    pub diagnostics_resolved: Vec<DiagnosticIdentity>,
    /// Whether new critical diagnostics were introduced (drives exit code 2).
    pub has_regressions: bool,
    /// Human-readable summary of changes.
    pub summary: Vec<String>,
}

/// A changed entity with before/after field values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityChange {
    /// Entity ID.
    pub id: String,
    /// List of changed fields with old and new values.
    pub changes: Vec<FieldChange>,
}

/// A single field-level change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldChange {
    /// Field name.
    pub field: String,
    /// Old value.
    pub old: String,
    /// New value.
    pub new: String,
}

/// Semantic identity of a diagnostic — stable across runs unlike sequential IDs.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct DiagnosticIdentity {
    /// Failure pattern category.
    pub pattern: String,
    /// Primary entity involved.
    pub entity: String,
    /// Source location.
    pub loc: String,
}

impl DiagnosticIdentity {
    /// Create a diagnostic identity from a diagnostic entry.
    fn from_entry(entry: &DiagnosticEntry) -> Self {
        Self {
            pattern: entry.pattern.clone(),
            entity: entry.entity.clone(),
            loc: entry.loc.clone(),
        }
    }
}

/// Run the `diff` command: compare two manifests and show structural changes.
///
/// Loads two manifest files (YAML or JSON), computes structural differences
/// across entities and diagnostics, and outputs the result. Exit code 2 means
/// new critical diagnostics were found (CI gate use case).
///
/// Returns the exit code: 0=no regressions, 2=new critical diagnostics.
pub fn run_diff(
    old_path: &Path,
    new_path: &Path,
    sections: &[String],
    format: OutputFormat,
    output_path: Option<&Path>,
) -> Result<u8, FlowspecError> {
    // Validate section names
    validate_sections(sections)?;

    // Load both manifests
    let old_manifest = load_manifest(old_path)?;
    let new_manifest = load_manifest(new_path)?;

    // Compute diff
    if let Some(reason) = diff_noncomparability(&old_manifest, &new_manifest) {
        let output = serde_json::json!({"execution":"incomplete","outcome":"incomplete","comparable":false,"reason":reason,"diagnostics_resolved":[],"entities_removed":[],"diagnostics_new":[],"entities_added":[],"entities_changed":[],"has_regressions":false,"summary":["reports are not comparable; absence cannot establish resolution"]});
        let rendered = match format {
            OutputFormat::Yaml => {
                serde_yaml::to_string(&output).map_err(|e| FlowspecError::Manifest {
                    reason: e.to_string(),
                })?
            }
            OutputFormat::Summary => format!("Diff incomplete: {reason}\n"),
            _ => serde_json::to_string(&output).map_err(|e| FlowspecError::Manifest {
                reason: e.to_string(),
            })?,
        };
        write_output(&rendered, output_path)?;
        return Ok(3);
    }
    let mut diff = compute_diff(&old_manifest, &new_manifest);

    // Apply section filter
    if !sections.is_empty() {
        apply_section_filter(&mut diff, sections);
    }

    let has_regressions = diff.has_regressions;

    // Format output
    let output = format_diff_result(&diff, format)?;

    write_output(&output, output_path)?;

    if has_regressions {
        Ok(2)
    } else {
        Ok(0)
    }
}

fn diff_noncomparability(old: &Manifest, new: &Manifest) -> Option<&'static str> {
    if old
        .execution
        .is_some_and(|e| e != crate::Execution::Complete)
        || new
            .execution
            .is_some_and(|e| e != crate::Execution::Complete)
    {
        return Some("input_execution_incomplete");
    }
    match (&old.authority, &new.authority) {
        (Some(a), Some(b)) => {
            for field in ["schema_version", "producer_contract", "scope"] {
                if a[field] != b[field] {
                    return Some("report_contract_or_scope_changed");
                }
            }
            for field in ["config", "adapters", "registry"] {
                if a["provenance"][field] != b["provenance"][field] {
                    return Some("configuration_or_adapter_changed");
                }
            }
            if a["producer"] != b["producer"] {
                return Some("producer_changed");
            }
            None
        }
        (None, None) => None,
        _ => Some("report_authority_missing"),
    }
}

/// Apply deterministic display pagination without changing identity, catalog or gate.
pub fn paginate_report(report: &mut crate::AnalysisReport, limit: Option<usize>, offset: usize) {
    if let Some(a) = report.authority.as_mut() {
        let indices = a["display"]["indices"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let total = indices.len();
        let begin = offset.min(total);
        let end = begin.saturating_add(limit.unwrap_or(total)).min(total);
        a["display"]["indices"] = serde_json::json!(&indices[begin..end]);
        a["continuation"]["shown"] = serde_json::json!(end - begin);
        a["continuation"]["matched_total"] = serde_json::json!(total);
        a["continuation"]["full"] = a["findings"]["total"].clone();
        a["continuation"]["truncated"] = serde_json::json!(end < total || begin > 0);
        a["continuation"]["next_cursor"] = if end < total {
            serde_json::json!(end)
        } else {
            serde_json::Value::Null
        };
    }
    report.diagnostics = report
        .diagnostics
        .iter()
        .skip(offset)
        .take(limit.unwrap_or(usize::MAX))
        .cloned()
        .collect();
    report.displayed_findings_total = report.diagnostics.len() as u64;
    report.metadata.diagnostic_count = report.diagnostics.len() as u64;
    recompute_diagnostic_summary(&report.diagnostics, &mut report.summary.diagnostic_summary);
}

/// Unified ordinary caller with report options and display paging.
#[allow(clippy::too_many_arguments)]
pub fn run_report_page(
    path: &Path,
    languages: &[String],
    config_path: Option<&Path>,
    format: OutputFormat,
    output_path: Option<&Path>,
    options: &crate::ReportOptions,
    analysis: &crate::AnalysisOptions,
    limit: Option<usize>,
    offset: usize,
) -> Result<u8, FlowspecError> {
    if limit == Some(0) {
        return Err(FlowspecError::Config {
            reason: "limit must be positive".into(),
            suggestion: "request at least one finding".into(),
        });
    }
    let canonical = resolve_path(path)?;
    let config = Config::load(&canonical, config_path)?;
    let mut result = prepare_report_with_analysis_options(
        &canonical,
        &config,
        &normalize_languages(languages),
        options,
        analysis,
    )?;
    paginate_report(&mut result.manifest, limit, offset);
    let output = render_report(&result.manifest, format)?;
    validate_manifest_size(&output, result.source_bytes, format_name(format))?;
    write_output(&output, output_path)?;
    Ok(crate::report_exit_code(&result.manifest, None))
}

/// Load a manifest from a file, detecting format by extension with fallback parsing.
///
/// Tries YAML first for `.yaml`/`.yml` extensions, JSON for `.json`.
/// Falls back to YAML-then-JSON parsing if extension is unrecognized.
pub fn load_manifest(path: &Path) -> Result<Manifest, FlowspecError> {
    if !path.exists() {
        return Err(FlowspecError::TargetNotFound {
            path: path.to_path_buf(),
        });
    }

    let content = std::fs::read_to_string(path).map_err(|e| FlowspecError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;

    if content.trim().is_empty() {
        return Err(FlowspecError::Manifest {
            reason: format!(
                "could not parse manifest at {}: file is empty",
                path.display()
            ),
        });
    }

    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();

    match ext.as_str() {
        "json" => {
            parse_json_manifest(&content, path).or_else(|_| parse_yaml_manifest(&content, path))
        }
        "yaml" | "yml" => {
            parse_yaml_manifest(&content, path).or_else(|_| parse_json_manifest(&content, path))
        }
        _ => parse_yaml_manifest(&content, path).or_else(|_| parse_json_manifest(&content, path)),
    }
}

/// Parse manifest content as YAML.
fn parse_yaml_manifest(content: &str, path: &Path) -> Result<Manifest, FlowspecError> {
    serde_yaml::from_str(content).map_err(|e| FlowspecError::Manifest {
        reason: format!("could not parse manifest at {}: {}", path.display(), e),
    })
}

/// Parse manifest content as JSON.
fn parse_json_manifest(content: &str, path: &Path) -> Result<Manifest, FlowspecError> {
    serde_json::from_str(content).map_err(|e| FlowspecError::Manifest {
        reason: format!("could not parse manifest at {}: {}", path.display(), e),
    })
}

/// Compute structural differences between two manifests.
pub fn compute_diff(old: &Manifest, new: &Manifest) -> DiffResult {
    if let Some(reason) = diff_noncomparability(old, new) {
        return DiffResult {
            entities_added: Vec::new(),
            entities_removed: Vec::new(),
            entities_changed: Vec::new(),
            diagnostics_new: Vec::new(),
            diagnostics_resolved: Vec::new(),
            has_regressions: false,
            summary: vec![format!("incomparable reports: {reason}")],
        };
    }
    // Entity diffing by ID
    let old_ids: HashSet<&str> = old.entities.iter().map(|e| e.id.as_str()).collect();
    let new_ids: HashSet<&str> = new.entities.iter().map(|e| e.id.as_str()).collect();

    let entities_added: Vec<String> = new_ids
        .difference(&old_ids)
        .map(|id| id.to_string())
        .collect();
    let entities_removed: Vec<String> = old_ids
        .difference(&new_ids)
        .map(|id| id.to_string())
        .collect();

    // Entity field-level changes for shared entities
    let old_entity_map: std::collections::HashMap<&str, &crate::manifest::types::EntityEntry> =
        old.entities.iter().map(|e| (e.id.as_str(), e)).collect();
    let new_entity_map: std::collections::HashMap<&str, &crate::manifest::types::EntityEntry> =
        new.entities.iter().map(|e| (e.id.as_str(), e)).collect();

    let mut entities_changed = Vec::new();
    for id in old_ids.intersection(&new_ids) {
        if let (Some(old_e), Some(new_e)) = (old_entity_map.get(id), new_entity_map.get(id)) {
            let mut changes = Vec::new();
            if old_e.kind != new_e.kind {
                changes.push(FieldChange {
                    field: "kind".to_string(),
                    old: old_e.kind.clone(),
                    new: new_e.kind.clone(),
                });
            }
            if old_e.vis != new_e.vis {
                changes.push(FieldChange {
                    field: "vis".to_string(),
                    old: old_e.vis.clone(),
                    new: new_e.vis.clone(),
                });
            }
            if old_e.sig != new_e.sig {
                changes.push(FieldChange {
                    field: "sig".to_string(),
                    old: old_e.sig.clone(),
                    new: new_e.sig.clone(),
                });
            }
            if old_e.loc != new_e.loc {
                changes.push(FieldChange {
                    field: "loc".to_string(),
                    old: old_e.loc.clone(),
                    new: new_e.loc.clone(),
                });
            }
            if !changes.is_empty() {
                entities_changed.push(EntityChange {
                    id: id.to_string(),
                    changes,
                });
            }
        }
    }

    // Diagnostic diffing by semantic identity (pattern + entity + loc)
    let old_diag_ids: HashSet<DiagnosticIdentity> = old
        .diagnostics
        .iter()
        .map(DiagnosticIdentity::from_entry)
        .collect();
    let new_diag_ids: HashSet<DiagnosticIdentity> = new
        .diagnostics
        .iter()
        .map(DiagnosticIdentity::from_entry)
        .collect();

    let diagnostics_new: Vec<DiagnosticIdentity> =
        new_diag_ids.difference(&old_diag_ids).cloned().collect();
    let diagnostics_resolved: Vec<DiagnosticIdentity> =
        old_diag_ids.difference(&new_diag_ids).cloned().collect();

    // Check for regressions: new diagnostics with critical severity
    let new_diag_set: HashSet<DiagnosticIdentity> = diagnostics_new.iter().cloned().collect();
    let has_regressions = new.diagnostics.iter().any(|d| {
        d.severity == "critical" && new_diag_set.contains(&DiagnosticIdentity::from_entry(d))
    });

    // Build summary
    let mut summary = Vec::new();
    if !entities_added.is_empty() {
        summary.push(format!("{} entities added", entities_added.len()));
    }
    if !entities_removed.is_empty() {
        summary.push(format!("{} entities removed", entities_removed.len()));
    }
    if !entities_changed.is_empty() {
        summary.push(format!("{} entities changed", entities_changed.len()));
    }
    if !diagnostics_new.is_empty() {
        summary.push(format!("{} new diagnostics", diagnostics_new.len()));
    }
    if !diagnostics_resolved.is_empty() {
        summary.push(format!(
            "{} diagnostics resolved",
            diagnostics_resolved.len()
        ));
    }
    if summary.is_empty() {
        summary.push("no changes".to_string());
    }

    DiffResult {
        entities_added,
        entities_removed,
        entities_changed,
        diagnostics_new,
        diagnostics_resolved,
        has_regressions,
        summary,
    }
}

/// Apply section filter to a DiffResult, clearing sections not in the filter.
fn apply_section_filter(diff: &mut DiffResult, sections: &[String]) {
    let section_set: HashSet<&str> = sections.iter().map(|s| s.as_str()).collect();

    if !section_set.contains("entities") {
        diff.entities_added.clear();
        diff.entities_removed.clear();
        diff.entities_changed.clear();
    }
    if !section_set.contains("diagnostics") {
        diff.diagnostics_new.clear();
        diff.diagnostics_resolved.clear();
        // Regressions only come from diagnostics section
        if !section_set.contains("diagnostics") {
            diff.has_regressions = false;
        }
    }

    // Rebuild summary after filtering
    let mut summary = Vec::new();
    if !diff.entities_added.is_empty() {
        summary.push(format!("{} entities added", diff.entities_added.len()));
    }
    if !diff.entities_removed.is_empty() {
        summary.push(format!("{} entities removed", diff.entities_removed.len()));
    }
    if !diff.entities_changed.is_empty() {
        summary.push(format!("{} entities changed", diff.entities_changed.len()));
    }
    if !diff.diagnostics_new.is_empty() {
        summary.push(format!("{} new diagnostics", diff.diagnostics_new.len()));
    }
    if !diff.diagnostics_resolved.is_empty() {
        summary.push(format!(
            "{} diagnostics resolved",
            diff.diagnostics_resolved.len()
        ));
    }
    if summary.is_empty() {
        summary.push("no changes".to_string());
    }
    diff.summary = summary;
}

/// Format a DiffResult for output.
fn format_diff_result(diff: &DiffResult, format: OutputFormat) -> Result<String, FlowspecError> {
    match format {
        OutputFormat::Yaml => serde_yaml::to_string(diff).map_err(|e| FlowspecError::Manifest {
            reason: format!("YAML serialization failed: {}", e),
        }),
        OutputFormat::Json => {
            serde_json::to_string_pretty(diff).map_err(|e| FlowspecError::Manifest {
                reason: format!("JSON serialization failed: {}", e),
            })
        }
        OutputFormat::Summary => {
            let mut lines = Vec::new();
            lines.push("Diff Summary:".to_string());
            for s in &diff.summary {
                lines.push(format!("  {}", s));
            }
            if !diff.entities_added.is_empty() {
                lines.push(String::new());
                lines.push("Entities added:".to_string());
                for id in &diff.entities_added {
                    lines.push(format!("  + {}", id));
                }
            }
            if !diff.entities_removed.is_empty() {
                lines.push(String::new());
                lines.push("Entities removed:".to_string());
                for id in &diff.entities_removed {
                    lines.push(format!("  - {}", id));
                }
            }
            if !diff.entities_changed.is_empty() {
                lines.push(String::new());
                lines.push("Entities changed:".to_string());
                for change in &diff.entities_changed {
                    lines.push(format!("  ~ {}", change.id));
                    for fc in &change.changes {
                        lines.push(format!("    {}: {} -> {}", fc.field, fc.old, fc.new));
                    }
                }
            }
            if !diff.diagnostics_new.is_empty() {
                lines.push(String::new());
                lines.push("New diagnostics:".to_string());
                for d in &diff.diagnostics_new {
                    lines.push(format!("  + {} on {} at {}", d.pattern, d.entity, d.loc));
                }
            }
            if !diff.diagnostics_resolved.is_empty() {
                lines.push(String::new());
                lines.push("Resolved diagnostics:".to_string());
                for d in &diff.diagnostics_resolved {
                    lines.push(format!("  - {} on {} at {}", d.pattern, d.entity, d.loc));
                }
            }
            Ok(lines.join("\n"))
        }
        OutputFormat::Sarif => Err(FlowspecError::FormatNotImplemented {
            format: "sarif".to_string(),
        }),
    }
}

/// Validate section names against the known manifest sections.
fn validate_sections(sections: &[String]) -> Result<(), FlowspecError> {
    for section in sections {
        if !VALID_SECTIONS.contains(&section.as_str()) {
            return Err(FlowspecError::Config {
                reason: format!("unknown section: {}", section),
                suggestion: format!("valid sections are: {}", VALID_SECTIONS.join(", ")),
            });
        }
    }
    Ok(())
}

/// Directories excluded from language detection during `init`.
const INIT_EXCLUDE_DIRS: &[&str] = &[
    "target",
    "node_modules",
    "__pycache__",
    ".git",
    ".flowspec",
    ".venv",
    "venv",
    "dist",
    "build",
    ".tox",
];

/// Run the `init` command: create `.flowspec/config.yaml` with sensible defaults.
///
/// Detects project languages by scanning for file extensions, creates the
/// `.flowspec/` directory and `config.yaml`, and prints the generated config
/// to stdout. If the config already exists, does nothing (no overwrite) and
/// exits 0.
///
/// Returns the exit code: 0 on success or already-exists, never 2.
pub fn run_init(path: &Path) -> Result<u8, FlowspecError> {
    let canonical = resolve_path(path)?;

    // Path must be a directory, not a file
    if !canonical.is_dir() {
        return Err(FlowspecError::Config {
            reason: format!("path is not a directory: {}", canonical.display()),
            suggestion: "provide a directory path, not a file".to_string(),
        });
    }

    let flowspec_dir = canonical.join(".flowspec");
    let config_path = flowspec_dir.join("config.yaml");

    // If config already exists, print it and exit 0 (no overwrite)
    if config_path.exists() {
        let existing = std::fs::read_to_string(&config_path).map_err(|e| FlowspecError::Io {
            path: config_path.clone(),
            source: e,
        })?;
        print!("{}", existing);
        return Ok(0);
    }

    // Create .flowspec/ directory
    std::fs::create_dir_all(&flowspec_dir).map_err(|e| FlowspecError::Io {
        path: flowspec_dir.clone(),
        source: e,
    })?;

    // Detect project languages
    let languages = detect_languages(&canonical);

    // Generate config YAML
    let config_content = generate_config_yaml(&languages);

    // Write config file
    std::fs::write(&config_path, &config_content).map_err(|e| FlowspecError::Io {
        path: config_path,
        source: e,
    })?;

    // Print to stdout (pipe-safe)
    print!("{}", config_content);

    Ok(0)
}

/// Detect project languages by scanning for file extensions.
///
/// Walks the project directory (skipping excluded directories like `target/`,
/// `node_modules/`, etc.) and collects unique languages based on file extensions.
/// Returns a sorted, deduplicated list of language names.
pub fn detect_languages(project_root: &Path) -> Vec<String> {
    let mut languages = HashSet::new();

    if let Ok(entries) = std::fs::read_dir(project_root) {
        scan_dir_for_languages(project_root, entries, &mut languages, 0);
    }

    let mut result: Vec<String> = languages.into_iter().collect();
    result.sort();
    result
}

/// Recursively scan a directory for source files, respecting exclusion list.
fn scan_dir_for_languages(
    _root: &Path,
    entries: std::fs::ReadDir,
    languages: &mut HashSet<String>,
    depth: usize,
) {
    // Limit recursion depth to avoid pathological cases
    if depth > 20 {
        return;
    }

    for entry in entries.flatten() {
        let path = entry.path();

        if path.is_dir() {
            // Check if this directory should be excluded
            if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
                if INIT_EXCLUDE_DIRS.contains(&dir_name) {
                    continue;
                }
            }
            // Recurse into subdirectory
            if let Ok(sub_entries) = std::fs::read_dir(&path) {
                scan_dir_for_languages(_root, sub_entries, languages, depth + 1);
            }
        } else if path.is_file() {
            if let Some(language) = crate::registry::language_id_for_path(&path) {
                languages.insert(language.to_string());
            }
        }
    }
}

/// Generate a default config.yaml content string with detected languages.
///
/// Produces well-commented YAML suitable for both human editing and machine parsing.
pub fn generate_config_yaml(languages: &[String]) -> String {
    let mut config = String::new();
    config.push_str("# Flowspec configuration\n");
    config.push_str("# Generated by flowspec init\n");
    config.push('\n');

    // Languages section
    config.push_str("# Languages to analyze (auto-detected)\n");
    config.push_str("languages:\n");
    if languages.is_empty() {
        config.push_str("  # No languages detected — add manually\n");
    } else {
        for lang in languages {
            config.push_str(&format!("  - {}\n", lang));
        }
    }
    config.push('\n');

    // Exclude section
    config.push_str("# Patterns to exclude from analysis\n");
    config.push_str("exclude:\n");
    config.push_str("  - \"target/\"\n");
    config.push_str("  - \"node_modules/\"\n");
    config.push_str("  - \"__pycache__/\"\n");
    config.push_str("  - \".git/\"\n");

    config
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
    crate::registry::Registry::default()
        .language(lang)
        .map(|d| d.id.clone())
        .unwrap_or_else(|| lang.to_string())
}

/// Validate a language name against v1 supported languages.
///
/// Accepts both canonical names and common abbreviations (e.g., "ts" for "typescript").
pub fn validate_language(lang: &str) -> Result<(), FlowspecError> {
    if crate::registry::Registry::default()
        .language(lang)
        .is_some()
    {
        Ok(())
    } else {
        Err(FlowspecError::UnsupportedLanguage {
            language: lang.to_string(),
        })
    }
}

/// Normalize a list of language arguments, expanding aliases.
pub fn normalize_languages(languages: &[String]) -> Vec<String> {
    languages.iter().map(|l| normalize_language(l)).collect()
}

/// Lossless normal form for graph records. Stable keys are derived from their
/// unchanged subjects and edge tuples rather than repeated in every record.
fn graph_tables(graph: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    use std::collections::BTreeMap;
    fn intern(tables: &mut BTreeMap<String, Vec<Value>>, name: &str, value: Value) -> usize {
        let rows = tables.entry(name.into()).or_default();
        if let Some(index) = rows.iter().position(|row| row == &value) {
            index
        } else {
            rows.push(value);
            rows.len() - 1
        }
    }
    let mut tables = BTreeMap::new();
    let mut entities = Vec::new();
    let mut ids = BTreeMap::new();
    for entity in graph["entities"].as_array()? {
        let subject = &entity["subject"];
        let path = subject["path_key"].as_str()?;
        let name = subject["qualified_name"].as_str()?;
        let decoded: Vec<u8> = (0..path.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&path[i..i + 2], 16).ok())
            .collect::<Option<_>>()?;
        let prefix = format!("{}::", std::str::from_utf8(&decoded).ok()?);
        let short = name.strip_prefix(&prefix);
        let range: Vec<Value> = [
            "start_byte",
            "end_byte",
            "start_line",
            "start_column",
            "end_line",
            "end_column",
        ]
        .iter()
        .map(|key| entity["range"][*key].clone())
        .collect();
        ids.insert(entity["key"].as_str()?.to_string(), entities.len());
        entities.push(json!([
            intern(&mut tables, "paths", json!(path)),
            intern(&mut tables, "languages", subject["language"].clone()),
            intern(
                &mut tables,
                "names",
                json!([short.is_some(), short.unwrap_or(name)])
            ),
            intern(&mut tables, "kinds", entity["kind"].clone()),
            intern(&mut tables, "ranges", json!(range))
        ]));
    }
    let mut edges = Vec::new();
    let mut edge_ids = BTreeMap::new();
    for edge in graph["dependency_graph"].as_array()? {
        if edge["evidence"] != json!([]) {
            return None;
        }
        edge_ids.insert(edge["key"].as_str()?.to_string(), edges.len());
        edges.push(json!([
            intern(&mut tables, "kinds", edge["kind"].clone()),
            ids.get(edge["source_key"].as_str()?)?,
            ids.get(edge["target_key"].as_str()?)?
        ]));
    }
    let boundaries: Vec<usize> = graph["boundaries"]
        .as_array()?
        .iter()
        .map(|edge| edge_ids.get(edge["key"].as_str()?).copied())
        .collect::<Option<_>>()?;
    let mut flows = Vec::new();
    for flow in graph["flows"].as_array()? {
        if flow["kind"] != "call" || flow["evidence"] != json!([]) {
            return None;
        }
        let nodes: Vec<usize> = flow["node_keys"]
            .as_array()?
            .iter()
            .map(|key| ids.get(key.as_str()?).copied())
            .collect::<Option<_>>()?;
        let edges: Vec<usize> = flow["edge_keys"]
            .as_array()?
            .iter()
            .map(|key| edge_ids.get(key.as_str()?).copied())
            .collect::<Option<_>>()?;
        flows.push(json!([nodes, edges]));
    }
    let packed = json!({"encoding":"flowspec.graph.tables.v1","tables":tables,
        "entities":entities,"dependency_graph":edges,"boundaries":boundaries,
        "flows":flows,"type_flows":graph["type_flows"],
        "compatibility_losses":"/authority/losses",
        "compatibility_pattern_state":"/authority/pattern_state"});
    // Never silently accept an encoding that fails to preserve every record.
    if expand_graph_tables(&packed).as_ref() != Some(graph) {
        return None;
    }
    let compact = graph_tables_v2(&packed)?;
    (expand_graph_tables(&compact).as_ref() == Some(graph)).then_some(compact)
}

/// Version 2 stores byte-exact UTF-8 paths, inline ranges, and single-edge
/// call paths by edge index. Version 1 remains a supported public input.
fn graph_tables_v2(v1: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    let mut packed = v1.clone();
    packed["encoding"] = json!("flowspec.graph.tables.v2");
    let paths: Vec<Value> = v1["tables"]["paths"]
        .as_array()?
        .iter()
        .map(|p| {
            let hex = p.as_str()?;
            if !hex.is_ascii() || hex.len() % 2 != 0 {
                return None;
            }
            let bytes: Vec<u8> = (0..hex.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).ok())
                .collect::<Option<_>>()?;
            // A tagged hex alternative preserves arbitrary POSIX path bytes.
            Some(match String::from_utf8(bytes) {
                Ok(path) => json!(path),
                Err(_) => json!(["hex", hex]),
            })
        })
        .collect::<Option<_>>()?;
    packed["tables"]["paths"] = json!(paths);
    let rows: Vec<Value> = v1["entities"]
        .as_array()?
        .iter()
        .map(|row| {
            let mut fields = row.as_array()?.get(..4)?.to_vec();
            fields.extend(
                v1["tables"]["ranges"]
                    .as_array()?
                    .get(row[4].as_u64()? as usize)?
                    .as_array()?
                    .iter()
                    .cloned(),
            );
            Some(json!(fields))
        })
        .collect::<Option<_>>()?;
    packed["entities"] = json!(rows);
    packed["tables"].as_object_mut()?.remove("ranges");
    let flows: Vec<Value> = v1["flows"]
        .as_array()?
        .iter()
        .map(|row| {
            let edges = row[1].as_array()?;
            if edges.len() == 1 {
                let edge = v1["dependency_graph"]
                    .as_array()?
                    .get(edges[0].as_u64()? as usize)?;
                if row[0] == json!([edge[1], edge[2]]) {
                    return Some(edges[0].clone());
                }
            }
            Some(row.clone())
        })
        .collect::<Option<_>>()?;
    packed["flows"] = json!(flows);
    Some(packed)
}

/// Restore the v1 row schema before applying its semantic decoder.
fn graph_tables_v1(v2: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    let mut packed = v2.clone();
    packed["encoding"] = json!("flowspec.graph.tables.v1");
    let paths: Vec<Value> = v2["tables"]["paths"]
        .as_array()?
        .iter()
        .map(|p| {
            if let Some(path) = p.as_str() {
                Some(json!(path
                    .as_bytes()
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect::<String>()))
            } else if p.as_array()?.len() == 2 && p[0] == "hex" {
                Some(json!(p[1].as_str()?))
            } else {
                None
            }
        })
        .collect::<Option<_>>()?;
    packed["tables"]["paths"] = json!(paths);
    let mut rows = Vec::new();
    let mut ranges = Vec::new();
    for row in v2["entities"].as_array()? {
        let fields = row.as_array()?;
        if fields.len() != 10 {
            return None;
        }
        ranges.push(json!(fields[4..].to_vec()));
        let mut original = fields[..4].to_vec();
        original.push(json!(ranges.len() - 1));
        rows.push(json!(original));
    }
    packed["entities"] = json!(rows);
    packed["tables"]["ranges"] = json!(ranges);
    let flows: Vec<Value> = v2["flows"]
        .as_array()?
        .iter()
        .map(|row| {
            if let Some(i) = row.as_u64() {
                let edge = v2["dependency_graph"].as_array()?.get(i as usize)?;
                Some(json!([[edge[1], edge[2]], [i]]))
            } else {
                Some(row.clone())
            }
        })
        .collect::<Option<_>>()?;
    packed["flows"] = json!(flows);
    Some(packed)
}

/// Expand the lossless graph normal form used by bounded reports.
pub fn expand_graph_tables(packed: &serde_json::Value) -> Option<serde_json::Value> {
    use serde_json::{json, Value};
    if packed["encoding"] == "flowspec.graph.tables.v2" {
        return expand_graph_tables(&graph_tables_v1(packed)?);
    }
    if packed["encoding"] != "flowspec.graph.tables.v1" {
        return None;
    }
    let tables = &packed["tables"];
    let lookup = |table: &str, index: &Value| {
        tables[table]
            .as_array()?
            .get(index.as_u64()? as usize)
            .cloned()
    };
    let mut entities = Vec::new();
    for row in packed["entities"].as_array()? {
        let path = lookup("paths", &row[0])?;
        let name = lookup("names", &row[2])?;
        let path_str = path.as_str()?;
        if path_str.len() % 2 != 0 || !path_str.is_ascii() {
            return None;
        }
        let bytes: Vec<u8> = (0..path_str.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&path.as_str().unwrap()[i..i + 2], 16).ok())
            .collect::<Option<_>>()?;
        let qualified = if name[0].as_bool()? {
            format!(
                "{}::{}",
                std::str::from_utf8(&bytes).ok()?,
                name[1].as_str()?
            )
        } else {
            name[1].as_str()?.into()
        };
        let kind = lookup("kinds", &row[3])?;
        let mut subject = json!({"path_key":path,"language":lookup("languages",&row[1])?,"qualified_name":qualified,"construct_kind":kind});
        let key = crate::identity::hash("binding", &subject);
        subject["binding_id"] = json!(key);
        let r = lookup("ranges", &row[4])?;
        let range = json!({"path_key":path,"start_byte":r[0],"end_byte":r[1],"start_line":r[2],"start_column":r[3],"end_line":r[4],"end_column":r[5]});
        entities.push(json!({"key":key,"kind":kind,"subject":subject,"range":range}));
    }
    let mut edges = Vec::new();
    for row in packed["dependency_graph"].as_array()? {
        let source = entities.get(row[1].as_u64()? as usize)?["key"].clone();
        let target = entities.get(row[2].as_u64()? as usize)?["key"].clone();
        let kind = lookup("kinds", &row[0])?;
        let key = crate::identity::hash(
            "binding",
            &json!({"kind":kind,"source_key":source,"target_key":target}),
        );
        edges.push(
            json!({"key":key,"kind":kind,"source_key":source,"target_key":target,"evidence":[]}),
        );
    }
    let boundaries: Vec<Value> = packed["boundaries"]
        .as_array()?
        .iter()
        .map(|i| edges.get(i.as_u64()? as usize).cloned())
        .collect::<Option<_>>()?;
    let mut flows = Vec::new();
    for row in packed["flows"].as_array()? {
        let nodes: Vec<Value> = row[0]
            .as_array()?
            .iter()
            .map(|i| entities.get(i.as_u64()? as usize).map(|e| e["key"].clone()))
            .collect::<Option<_>>()?;
        let edge_keys: Vec<Value> = row[1]
            .as_array()?
            .iter()
            .map(|i| edges.get(i.as_u64()? as usize).map(|e| e["key"].clone()))
            .collect::<Option<_>>()?;
        let key = crate::identity::hash(
            "binding",
            &json!({"kind":"call","node_keys":nodes,"edge_keys":edge_keys}),
        );
        flows.push(
            json!({"key":key,"kind":"call","node_keys":nodes,"edge_keys":edge_keys,"evidence":[]}),
        );
    }
    Some(
        json!({"entities":entities,"dependency_graph":edges,"boundaries":boundaries,"flows":flows,"type_flows":packed["type_flows"]}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::types::{DiagnosticEntry, EntityEntry, Manifest};

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

    // --- Diff command unit test helpers ---

    /// Create a DiagnosticEntry with specified fields for diff testing.
    fn diag_entry(pattern: &str, entity: &str, loc: &str, severity: &str) -> DiagnosticEntry {
        DiagnosticEntry {
            id: "D001".to_string(),
            pattern: pattern.to_string(),
            severity: severity.to_string(),
            confidence: "high".to_string(),
            entity: entity.to_string(),
            message: String::new(),
            evidence: vec![],
            suggestion: String::new(),
            loc: loc.to_string(),
        }
    }

    /// Create a Manifest with specified entities and diagnostics.
    fn manifest_with(entities: Vec<EntityEntry>, diagnostics: Vec<DiagnosticEntry>) -> Manifest {
        let mut m = Manifest::empty();
        m.entities = entities;
        m.diagnostics = diagnostics;
        m
    }

    // --- T1: compute_diff — identical empty manifests ---
    #[test]
    fn compute_diff_identical_empty() {
        let old = Manifest::empty();
        let new = Manifest::empty();
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_added.is_empty());
        assert!(diff.entities_removed.is_empty());
        assert!(diff.entities_changed.is_empty());
        assert!(diff.diagnostics_new.is_empty());
        assert!(diff.diagnostics_resolved.is_empty());
        assert!(!diff.has_regressions);
        assert_eq!(diff.summary, vec!["no changes"]);
    }

    // --- T2: compute_diff — entity added ---
    #[test]
    fn compute_diff_entity_added() {
        let old = manifest_with(vec![entity_with_id("A")], vec![]);
        let new = manifest_with(vec![entity_with_id("A"), entity_with_id("B")], vec![]);
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_added.contains(&"B".to_string()));
        assert!(diff.entities_removed.is_empty());
        assert!(diff.entities_changed.is_empty());
    }

    // --- T3: compute_diff — entity removed ---
    #[test]
    fn compute_diff_entity_removed() {
        let old = manifest_with(vec![entity_with_id("A"), entity_with_id("B")], vec![]);
        let new = manifest_with(vec![entity_with_id("A")], vec![]);
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_removed.contains(&"B".to_string()));
        assert!(diff.entities_added.is_empty());
    }

    // --- T4: compute_diff — entity field change (sig) ---
    #[test]
    fn compute_diff_entity_changed_sig() {
        let mut old_e = entity_with_id("A");
        old_e.sig = String::new();
        let mut new_e = entity_with_id("A");
        new_e.sig = "(i32) -> bool".to_string();
        let old = manifest_with(vec![old_e], vec![]);
        let new = manifest_with(vec![new_e], vec![]);
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_added.is_empty());
        assert!(diff.entities_removed.is_empty());
        assert_eq!(diff.entities_changed.len(), 1);
        let change = &diff.entities_changed[0];
        assert_eq!(change.id, "A");
        assert_eq!(change.changes.len(), 1);
        assert_eq!(change.changes[0].field, "sig");
        assert_eq!(change.changes[0].old, "");
        assert_eq!(change.changes[0].new, "(i32) -> bool");
    }

    // --- T5: compute_diff — mixed changes (add + remove + change) ---
    #[test]
    fn compute_diff_mixed_changes() {
        let mut old_a = entity_with_id("A");
        old_a.vis = "pub".to_string();
        let old = manifest_with(vec![old_a, entity_with_id("B")], vec![]);
        let mut new_a = entity_with_id("A");
        new_a.vis = "priv".to_string();
        let new = manifest_with(vec![new_a, entity_with_id("C")], vec![]);
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_added.contains(&"C".to_string()));
        assert!(diff.entities_removed.contains(&"B".to_string()));
        assert_eq!(diff.entities_changed.len(), 1);
        assert_eq!(diff.entities_changed[0].id, "A");
        let summary_str = diff.summary.join(" ");
        assert!(summary_str.contains("added"), "summary: {}", summary_str);
        assert!(summary_str.contains("removed"), "summary: {}", summary_str);
        assert!(summary_str.contains("changed"), "summary: {}", summary_str);
    }

    // --- T6: compute_diff — new critical diagnostic triggers has_regressions ---
    #[test]
    fn compute_diff_new_critical_regression() {
        let old = manifest_with(vec![], vec![]);
        let new = manifest_with(
            vec![],
            vec![diag_entry(
                "contract_mismatch",
                "foo",
                "test.py:1",
                "critical",
            )],
        );
        let diff = compute_diff(&old, &new);
        assert!(
            diff.has_regressions,
            "Critical diagnostic must trigger has_regressions"
        );
        assert_eq!(diff.diagnostics_new.len(), 1);
    }

    // --- T7: compute_diff — new warning does NOT trigger regressions ---
    #[test]
    fn compute_diff_new_warning_no_regression() {
        let old = manifest_with(vec![], vec![]);
        let new = manifest_with(
            vec![],
            vec![diag_entry("data_dead_end", "bar", "test.py:5", "warning")],
        );
        let diff = compute_diff(&old, &new);
        assert!(
            !diff.has_regressions,
            "Warning diagnostic must not trigger has_regressions"
        );
        assert_eq!(diff.diagnostics_new.len(), 1);
    }

    // --- T8: compute_diff — resolved diagnostic detected ---
    #[test]
    fn compute_diff_diagnostic_resolved() {
        let old = manifest_with(
            vec![],
            vec![diag_entry("phantom_dependency", "os", "main.py:2", "info")],
        );
        let new = manifest_with(vec![], vec![]);
        let diff = compute_diff(&old, &new);
        assert_eq!(diff.diagnostics_resolved.len(), 1);
        assert_eq!(diff.diagnostics_resolved[0].pattern, "phantom_dependency");
        assert!(diff.diagnostics_new.is_empty());
        assert!(!diff.has_regressions);
    }

    // --- T9: load_manifest — valid YAML ---
    #[test]
    fn load_manifest_valid_yaml() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.yaml");
        let manifest = Manifest::empty();
        let yaml = serde_yaml::to_string(&manifest).unwrap();
        std::fs::write(&path, &yaml).unwrap();
        let result = load_manifest(&path);
        assert!(
            result.is_ok(),
            "load_manifest YAML failed: {:?}",
            result.err()
        );
    }

    // --- T10: load_manifest — valid JSON ---
    #[test]
    fn load_manifest_valid_json() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("test.json");
        let manifest = Manifest::empty();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        std::fs::write(&path, &json).unwrap();
        let result = load_manifest(&path);
        assert!(
            result.is_ok(),
            "load_manifest JSON failed: {:?}",
            result.err()
        );
    }

    // --- T11: load_manifest — empty file returns error ---
    #[test]
    fn load_manifest_empty_file_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("empty.yaml");
        std::fs::write(&path, "   \n  ").unwrap();
        let result = load_manifest(&path);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("file is empty"),
            "Error should mention 'file is empty': {}",
            err_msg
        );
    }

    // --- T12: load_manifest — nonexistent path returns TargetNotFound ---
    #[test]
    fn load_manifest_nonexistent_path() {
        let path = Path::new("/tmp/nonexistent_manifest_test_12345.yaml");
        let result = load_manifest(path);
        assert!(result.is_err());
        match result.unwrap_err() {
            FlowspecError::TargetNotFound { path: p } => {
                assert_eq!(p, path.to_path_buf());
            }
            other => panic!("Expected TargetNotFound, got: {:?}", other),
        }
    }

    // --- T13: apply_section_filter — entities only ---
    #[test]
    fn apply_section_filter_entities_only() {
        let mut diff = DiffResult {
            entities_added: vec!["A".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "data_dead_end".to_string(),
                entity: "foo".to_string(),
                loc: "x.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![
                "1 entities added".to_string(),
                "1 new diagnostics".to_string(),
            ],
        };
        apply_section_filter(&mut diff, &["entities".to_string()]);
        assert_eq!(diff.entities_added, vec!["A".to_string()]);
        assert!(diff.diagnostics_new.is_empty());
        assert!(diff.diagnostics_resolved.is_empty());
        assert!(
            !diff.has_regressions,
            "has_regressions must be false when diagnostics filtered out"
        );
    }

    // --- T14: apply_section_filter — diagnostics only ---
    #[test]
    fn apply_section_filter_diagnostics_only() {
        let mut diff = DiffResult {
            entities_added: vec!["A".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "data_dead_end".to_string(),
                entity: "foo".to_string(),
                loc: "x.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![],
        };
        apply_section_filter(&mut diff, &["diagnostics".to_string()]);
        assert!(diff.entities_added.is_empty());
        assert!(diff.entities_removed.is_empty());
        assert!(diff.entities_changed.is_empty());
        assert_eq!(diff.diagnostics_new.len(), 1);
        assert!(
            diff.has_regressions,
            "has_regressions must be preserved for diagnostics filter"
        );
    }

    // --- T15: apply_section_filter — both sections preserves all ---
    #[test]
    fn apply_section_filter_both_sections() {
        let mut diff = DiffResult {
            entities_added: vec!["A".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "data_dead_end".to_string(),
                entity: "foo".to_string(),
                loc: "x.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![],
        };
        apply_section_filter(
            &mut diff,
            &["entities".to_string(), "diagnostics".to_string()],
        );
        assert_eq!(diff.entities_added.len(), 1);
        assert_eq!(diff.diagnostics_new.len(), 1);
        assert!(diff.has_regressions);
    }

    // --- T16: apply_section_filter — empty clears all ---
    #[test]
    fn apply_section_filter_empty_clears_all() {
        let mut diff = DiffResult {
            entities_added: vec!["X".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "test".to_string(),
                entity: "e".to_string(),
                loc: "l".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![],
        };
        apply_section_filter(&mut diff, &[]);
        assert!(diff.entities_added.is_empty());
        assert!(diff.diagnostics_new.is_empty());
        assert!(!diff.has_regressions);
        assert_eq!(diff.summary, vec!["no changes"]);
    }

    // --- T17: validate_sections — all valid section names accepted ---
    #[test]
    fn validate_sections_all_valid_accepted() {
        for &name in VALID_SECTIONS {
            assert!(
                validate_sections(&[name.to_string()]).is_ok(),
                "Section '{}' should be accepted",
                name
            );
        }
    }

    // --- T18: validate_sections — unknown rejected with helpful error ---
    #[test]
    fn validate_sections_unknown_rejected() {
        let result = validate_sections(&["nonexistent".to_string()]);
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("unknown section: nonexistent"),
            "Error must mention unknown section: {}",
            err_msg
        );
    }

    // --- T19: validate_sections — empty list accepted ---
    #[test]
    fn validate_sections_empty_accepted() {
        assert!(validate_sections(&[]).is_ok());
    }

    // --- T20: DiffResult serializes to YAML ---
    #[test]
    fn diff_result_serializes_yaml() {
        let diff = DiffResult {
            entities_added: vec!["new_fn".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec!["1 entities added".to_string()],
        };
        let yaml = serde_yaml::to_string(&diff).unwrap();
        assert!(yaml.contains("entities_added"));
        assert!(yaml.contains("new_fn"));
    }

    // --- T21: Empty DiffResult serializes cleanly ---
    #[test]
    fn diff_result_empty_serializes_cleanly() {
        let diff = DiffResult {
            entities_added: vec![],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec!["no changes".to_string()],
        };
        let yaml = serde_yaml::to_string(&diff);
        assert!(yaml.is_ok(), "Empty DiffResult YAML: {:?}", yaml.err());
        let json = serde_json::to_string_pretty(&diff);
        assert!(json.is_ok(), "Empty DiffResult JSON: {:?}", json.err());
    }

    // --- T22: DiffResult round-trips through JSON ---
    #[test]
    fn diff_result_json_roundtrip() {
        let diff = DiffResult {
            entities_added: vec!["A".to_string()],
            entities_removed: vec!["B".to_string()],
            entities_changed: vec![EntityChange {
                id: "C".to_string(),
                changes: vec![FieldChange {
                    field: "vis".to_string(),
                    old: "pub".to_string(),
                    new: "priv".to_string(),
                }],
            }],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "data_dead_end".to_string(),
                entity: "foo".to_string(),
                loc: "test.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec!["1 entities added".to_string()],
        };
        let json = serde_json::to_string(&diff).unwrap();
        let deserialized: DiffResult = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.entities_added, diff.entities_added);
        assert_eq!(deserialized.entities_removed, diff.entities_removed);
        assert_eq!(deserialized.entities_changed.len(), 1);
        assert_eq!(deserialized.entities_changed[0].id, "C");
        assert_eq!(deserialized.diagnostics_new.len(), 1);
        assert_eq!(deserialized.diagnostics_new[0].pattern, "data_dead_end");
    }

    // --- T23: format_diff_result — YAML format ---
    #[test]
    fn format_diff_result_yaml() {
        let diff = DiffResult {
            entities_added: vec!["new_entity".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec!["1 entities added".to_string()],
        };
        let result = format_diff_result(&diff, OutputFormat::Yaml);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("entities_added"));
        assert!(output.contains("new_entity"));
    }

    // --- T24: format_diff_result — Summary structure ---
    #[test]
    fn format_diff_result_summary_structure() {
        let diff = DiffResult {
            entities_added: vec!["fn_a".to_string()],
            entities_removed: vec!["fn_b".to_string()],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "data_dead_end".to_string(),
                entity: "fn_c".to_string(),
                loc: "x.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec![
                "1 entities added".to_string(),
                "1 entities removed".to_string(),
                "1 new diagnostics".to_string(),
            ],
        };
        let result = format_diff_result(&diff, OutputFormat::Summary);
        assert!(result.is_ok());
        let output = result.unwrap();
        assert!(output.contains("Diff Summary:"), "output: {}", output);
        assert!(output.contains("Entities added:"), "output: {}", output);
        assert!(output.contains("+ fn_a"), "output: {}", output);
        assert!(output.contains("Entities removed:"), "output: {}", output);
        assert!(output.contains("- fn_b"), "output: {}", output);
        assert!(output.contains("New diagnostics:"), "output: {}", output);
        assert!(
            output.contains("data_dead_end on fn_c"),
            "output: {}",
            output
        );
    }

    // --- T25: format_diff_result — SARIF returns FormatNotImplemented ---
    #[test]
    fn format_diff_result_sarif_not_implemented() {
        let diff = DiffResult {
            entities_added: vec![],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![],
            diagnostics_resolved: vec![],
            has_regressions: false,
            summary: vec!["no changes".to_string()],
        };
        let result = format_diff_result(&diff, OutputFormat::Sarif);
        assert!(result.is_err());
        match result.unwrap_err() {
            FlowspecError::FormatNotImplemented { format } => {
                assert_eq!(format, "sarif");
            }
            other => panic!("Expected FormatNotImplemented, got: {:?}", other),
        }
    }

    // --- T29: compute_diff — identical non-empty manifests ---
    #[test]
    fn compute_diff_identical_nonempty() {
        let entities = vec![
            entity_with_id("A"),
            entity_with_id("B"),
            entity_with_id("C"),
        ];
        let diagnostics = vec![
            diag_entry("data_dead_end", "A", "a.py:1", "warning"),
            diag_entry("phantom_dependency", "os", "b.py:2", "info"),
        ];
        let old = manifest_with(entities.clone(), diagnostics.clone());
        let new = manifest_with(entities, diagnostics);
        let diff = compute_diff(&old, &new);
        assert!(diff.entities_added.is_empty());
        assert!(diff.entities_removed.is_empty());
        assert!(diff.entities_changed.is_empty());
        assert!(diff.diagnostics_new.is_empty());
        assert!(diff.diagnostics_resolved.is_empty());
        assert!(!diff.has_regressions);
        assert_eq!(diff.summary, vec!["no changes"]);
    }

    // --- T30: section filter with unimplemented section produces empty output ---
    #[test]
    fn section_filter_unimplemented_section_empty_output() {
        let mut diff = DiffResult {
            entities_added: vec!["A".to_string()],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "test".to_string(),
                entity: "e".to_string(),
                loc: "l".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![],
        };
        apply_section_filter(&mut diff, &["flows".to_string()]);
        assert!(diff.entities_added.is_empty());
        assert!(diff.diagnostics_new.is_empty());
        assert!(!diff.has_regressions);
        assert_eq!(diff.summary, vec!["no changes"]);
    }

    // --- T31: redundant condition — filtering out diagnostics clears has_regressions ---
    #[test]
    fn apply_section_filter_regression_redundant_condition() {
        let mut diff = DiffResult {
            entities_added: vec![],
            entities_removed: vec![],
            entities_changed: vec![],
            diagnostics_new: vec![DiagnosticIdentity {
                pattern: "contract_mismatch".to_string(),
                entity: "foo".to_string(),
                loc: "x.py:1".to_string(),
            }],
            diagnostics_resolved: vec![],
            has_regressions: true,
            summary: vec![],
        };
        apply_section_filter(&mut diff, &["entities".to_string()]);
        assert!(
            !diff.has_regressions,
            "Filtering out diagnostics must clear has_regressions"
        );
    }

    // --- T32: entity with all 4 fields changed ---
    #[test]
    fn compute_diff_entity_all_fields_changed() {
        let old_e = EntityEntry {
            id: "A".to_string(),
            kind: "fn".to_string(),
            vis: "pub".to_string(),
            sig: "()".to_string(),
            loc: "a.py:1".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        };
        let new_e = EntityEntry {
            id: "A".to_string(),
            kind: "method".to_string(),
            vis: "priv".to_string(),
            sig: "(self, i32) -> bool".to_string(),
            loc: "b.py:42".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        };
        let old = manifest_with(vec![old_e], vec![]);
        let new = manifest_with(vec![new_e], vec![]);
        let diff = compute_diff(&old, &new);
        assert_eq!(diff.entities_changed.len(), 1);
        let change = &diff.entities_changed[0];
        assert_eq!(change.id, "A");
        assert_eq!(
            change.changes.len(),
            4,
            "All 4 fields should be changed: {:?}",
            change.changes
        );
        let field_names: Vec<&str> = change.changes.iter().map(|c| c.field.as_str()).collect();
        assert!(field_names.contains(&"kind"));
        assert!(field_names.contains(&"vis"));
        assert!(field_names.contains(&"sig"));
        assert!(field_names.contains(&"loc"));
    }
}

/// One exact symbol identity and source site in a trace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceSymbol {
    pub binding_id: String,
    pub qualified_name: String,
    pub location: crate::parser::ir::Location,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceHop {
    pub symbol: TraceSymbol,
    pub edge_kind: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExactTracePath {
    pub id: String,
    pub entry: TraceSymbol,
    pub hops: Vec<TraceHop>,
    pub cyclic: bool,
    pub truncated: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceOutput {
    pub schema_version: u32,
    pub producer: serde_json::Value,
    pub scope: serde_json::Value,
    pub execution: String,
    pub outcome: String,
    pub symbol: String,
    pub direction: String,
    pub paths: Vec<ExactTracePath>,
    pub unresolved: Vec<crate::parser::ir::Location>,
    pub continuation: Vec<String>,
    pub losses: Vec<serde_json::Value>,
    pub losses_total: usize,
    pub paths_enumerated: usize,
    pub unresolved_total: usize,
    pub continuation_total: usize,
    pub details_truncated: bool,
    pub next_path: Option<String>,
}
/// Same graph and loss policy for library, CLI and MCP trace consumers.
pub fn exact_trace_output(
    root: &Path,
    result: &crate::AnalysisResult,
    symbol: &str,
    depth: usize,
    direction: TraceDirection,
) -> Result<TraceOutput, FlowspecError> {
    let graph = &result.graph;
    let name = find_matching_symbol(symbol, &result.manifest.entities)?;
    let mut matches = result
        .graph
        .all_symbols()
        .filter(|(_, s)| s.qualified_name == name);
    let (id, _) = matches
        .next()
        .ok_or_else(|| FlowspecError::SymbolNotFound(name.clone()))?;
    if matches.next().is_some() {
        return Err(FlowspecError::SymbolNotFound(format!(
            "Ambiguous binding: {name}"
        )));
    }
    let directions: Vec<_> = match direction {
        TraceDirection::Forward => vec![false],
        TraceDirection::Backward => vec![true],
        TraceDirection::Both => vec![false, true],
    };
    let identity = |id: SymbolId| -> Option<TraceSymbol> {
        let s = result.graph.get_symbol(id)?;
        let mut location = s.location.clone();
        location.file = s
            .location
            .file
            .strip_prefix(root)
            .unwrap_or(&s.location.file)
            .to_path_buf();
        let language = crate::registry::Registry::default()
            .language_for_path(&s.location.file)
            .map(|d| d.id.clone());
        let base = serde_json::json!({"path_key":crate::report_authority::path_key(root,&s.location.file),"language":language,"qualified_name":s.qualified_name,"construct_kind":format!("{:?}",s.kind).to_lowercase()});
        Some(TraceSymbol {
            binding_id: crate::report_authority::hash("binding", &base),
            qualified_name: s.qualified_name.clone(),
            location,
        })
    };
    let mut paths = vec![];
    let mut unresolved = vec![];
    let mut continuation = vec![];
    let mut incomplete = false;
    let mut seen = HashSet::new();
    for backward in directions {
        let report = crate::analyzer::flow::trace_report(&result.graph, id, depth, backward);
        incomplete |= report.incomplete;
        unresolved.extend(report.unresolved.into_iter().map(|mut location| {
            location.file = location
                .file
                .strip_prefix(root)
                .unwrap_or(&location.file)
                .to_path_buf();
            location
        }));
        let mut occurrences = std::collections::HashMap::<String, usize>::new();
        for path in report.paths {
            let Some(entry) = identity(path.entry) else {
                continue;
            };
            let hops: Vec<_> = path
                .steps
                .iter()
                .filter_map(|step| {
                    identity(step.symbol).map(|symbol| TraceHop {
                        symbol,
                        edge_kind: format!("{:?}", step.edge_kind).to_lowercase(),
                    })
                })
                .collect();
            let key = crate::report_authority::hash(
                "trace-path",
                &serde_json::json!({"entry":entry.binding_id,"hops":hops,"backward":backward}),
            );
            // Equal binding hops can arise from distinct source call sites.
            // Reconstruct the actual edge-reference combination in traversal
            // order, without changing the public Graph or trace struct layout.
            let occurrence = occurrences.entry(key.clone()).or_default();
            let mut current = path.entry;
            let mut choices = Vec::new();
            for step in &path.steps {
                let edges = if backward {
                    graph.edges_to(current)
                } else {
                    graph.edges_from(current)
                };
                let references: Vec<_> = edges
                    .iter()
                    .filter(|e| e.target == step.symbol && e.kind == step.edge_kind)
                    .filter_map(|e| e.reference_id.and_then(|r| graph.get_reference(r)))
                    .map(|r| {
                        let mut location = r.location.clone();
                        location.file = location
                            .file
                            .strip_prefix(root)
                            .unwrap_or(&location.file)
                            .to_path_buf();
                        location
                    })
                    .collect();
                choices.push(references);
                current = step.symbol;
            }
            let mut index = *occurrence;
            *occurrence += 1;
            let mut sites = Vec::new();
            for references in choices.into_iter().rev() {
                if !references.is_empty() {
                    sites.push(references[index % references.len()].clone());
                    index /= references.len();
                }
            }
            sites.reverse();
            let key = crate::report_authority::hash(
                "trace-path",
                &serde_json::json!({"binding_path":key,"source_references":sites}),
            );
            if !seen.insert(key.clone()) {
                continue;
            }
            if path.truncated {
                continuation.push(key.clone());
            }
            paths.push(ExactTracePath {
                id: key,
                entry,
                hops,
                cyclic: path.is_cyclic,
                truncated: path.truncated,
            });
        }
    }
    paths.sort_by(|a, b| a.id.cmp(&b.id));
    let mut sites = vec![result
        .graph
        .get_symbol(id)
        .expect("matched trace entry")
        .location
        .clone()];
    sites.extend(paths.iter().flat_map(|p| p.hops.iter()).map(|hop| {
        let mut location = hop.symbol.location.clone();
        location.file = root.join(&location.file);
        location
    }));
    let mut losses = Vec::new();
    for loss in result
        .manifest
        .authority
        .as_ref()
        .and_then(|a| a["losses"].as_array())
        .into_iter()
        .flatten()
    {
        if loss["reason"] == "output_truncation" {
            continue;
        }
        let required: Vec<_> = loss["required_capabilities"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(serde_json::Value::as_str)
            .collect();
        if !required.is_empty()
            && !required.iter().any(|cap| {
                matches!(
                    *cap,
                    "calls"
                        | "imports"
                        | "resolution"
                        | "dispatch"
                        | "parser_traversal"
                        | "symbols"
                        | "scopes"
                        | "name_use"
                )
            })
        {
            continue;
        }
        let affected = sites.iter().any(|site| {
            let key = crate::report_authority::path_key(root, &site.file);
            let path_matches = loss["scope_paths"]
                .as_array()
                .is_some_and(|keys| keys.iter().any(|k| k == &key));
            let ranges: Vec<_> = loss["ranges"].as_array().into_iter().flatten().collect();
            path_matches
                && (ranges.is_empty()
                    || ranges.iter().any(|range| {
                        range["start_line"].as_u64().unwrap_or(0) <= site.end_line as u64
                            && range["end_line"].as_u64().unwrap_or(u64::MAX) >= site.line as u64
                    }))
        });
        if affected {
            incomplete = true;
            losses.push(loss.clone());
        }
    }
    unresolved.sort_by(|a, b| {
        (&a.file, a.line, a.column, a.end_line, a.end_column).cmp(&(
            &b.file,
            b.line,
            b.column,
            b.end_line,
            b.end_column,
        ))
    });
    unresolved.dedup();
    let mut output = TraceOutput {
        schema_version: 2,
        producer: result
            .manifest
            .authority
            .as_ref()
            .map(|a| a["producer"].clone())
            .unwrap_or_default(),
        scope: result
            .manifest
            .authority
            .as_ref()
            .map(|a| a["scope"].clone())
            .unwrap_or_default(),
        execution: if incomplete { "incomplete" } else { "complete" }.into(),
        outcome: if incomplete { "incomplete" } else { "clean" }.into(),
        symbol: name,
        direction: format!("{direction:?}").to_lowercase(),
        losses_total: losses.len(),
        paths_enumerated: paths.len(),
        unresolved_total: unresolved.len(),
        continuation_total: continuation.len(),
        details_truncated: false,
        next_path: None,
        paths,
        unresolved,
        continuation,
        losses,
    };
    // Bound once for all consumers, allowing MCP to carry equal text + structured content.
    loop {
        let json = serde_json::to_vec(&output).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        })?;
        let yaml = serde_yaml::to_string(&output).map_err(|e| FlowspecError::Manifest {
            reason: e.to_string(),
        })?;
        if json.len() <= 9_000 && yaml.len() <= 9_000 {
            break;
        }
        output.details_truncated = true;
        output.execution = "incomplete".into();
        output.outcome = "incomplete".into();
        if let Some(path) = output.paths.pop() {
            if output.next_path.is_none() {
                output.next_path = Some(path.id.clone());
            }
            if !output.continuation.contains(&path.id) {
                output.continuation_total += 1;
                output.continuation.push(path.id);
            }
        } else if !output.losses.is_empty() {
            output.losses.pop();
        } else if !output.unresolved.is_empty() {
            output.unresolved.pop();
        } else if output.continuation.len() > 1 {
            output.continuation.pop();
        } else {
            return Err(FlowspecError::Manifest {
                reason: "trace scope/provenance exceeds the output byte budget".into(),
            });
        }
    }
    Ok(output)
}
#[allow(clippy::too_many_arguments)]
pub fn run_trace_report(
    path: &Path,
    symbol: &str,
    languages: &[String],
    depth: usize,
    direction: TraceDirection,
    format: OutputFormat,
    output: Option<&Path>,
    config_path: Option<&Path>,
) -> Result<u8, FlowspecError> {
    let root = resolve_path(path)?;
    let config = Config::load(&root, config_path)?;
    let result = crate::analyze(&root, &config, &normalize_languages(languages))?;
    let trace = exact_trace_output(&root, &result, symbol, depth, direction)?;
    let text=match format {
        OutputFormat::Json=>serde_json::to_string(&trace),
        OutputFormat::Yaml=>return {let text=serde_yaml::to_string(&trace).map_err(|e|FlowspecError::Manifest{reason:e.to_string()})?;write_output(&text,output)?;Ok(if trace.execution=="incomplete"{3}else{0})},
        OutputFormat::Summary=>Ok(format!("Trace: {}\nExecution: {}\nPaths: {}\nUnresolved: {}\nContinuation: {}\n",trace.symbol,trace.execution,trace.paths.len(),trace.unresolved.len(),trace.continuation.len())),
        OutputFormat::Sarif=>Ok(serde_json::json!({"version":"2.1.0","runs":[{"tool":{"driver":{"name":"flowspec"}},"invocations":[{"executionSuccessful":trace.execution=="complete","properties":{"flowspec":trace}}],"results":[]}]}).to_string()),
    }.map_err(|e|FlowspecError::Manifest{reason:e.to_string()})?;
    write_output(&text, output)?;
    Ok(if trace.execution == "incomplete" {
        3
    } else {
        0
    })
}
