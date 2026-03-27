//! Flowspec — static code analyzer that traces the flow of all data in a codebase.
//!
//! Optimized for efficient use by AI agents during building, debugging,
//! and within CI workflows. Data-oriented architecture inspired by ECS:
//! symbols are IDs in flat tables, analyzers are functions that query
//! the graph, manifests are exports of the analysis graph.
//!
//! # Architecture
//!
//! ```text
//! Source files → Parser (tree-sitter) → IR → Graph → Cross-file Resolution → Analyzers → Manifest
//! ```
//!
//! Analysis is two-pass: first, all files are parsed and their IR is inserted
//! into the graph via `populate_graph`. Then `resolve_cross_file_imports`
//! links import symbols to definitions in other files using a module-to-file
//! mapping built by `build_module_map`.
//!
//! The graph is the source of truth. Manifests are exports optimized
//! for different consumers (YAML for agents, JSON for tools, SARIF for CI,
//! summary for humans).

/// Diagnostic detection, flow tracing, and boundary analysis.
pub mod analyzer;
/// Extracted CLI command logic — testable library functions for analyze, diagnose, trace, diff, init.
pub mod commands;
/// Configuration loading and validation.
pub mod config;
/// Library-level error types — `FlowspecError` and `ManifestError`.
pub mod error;
/// Persistent in-memory analysis graph — flat symbol tables with bidirectional edges.
pub mod graph;
/// Output formatting — YAML, JSON, SARIF, summary. One formatter per output format.
pub mod manifest;
/// Tree-sitter parsing and language adapters — Python, JavaScript/TypeScript, Rust.
pub mod parser;

#[cfg(test)]
pub mod test_utils;

#[cfg(test)]
mod pipeline_tests;

#[cfg(test)]
mod pattern_integration_tests;

#[cfg(test)]
mod cycle10_surface_tests;

#[cfg(test)]
mod cycle10_js_cross_file_tests;

#[cfg(test)]
mod cycle11_rust_call_tests;

#[cfg(test)]
mod cycle11_surface_tests;

#[cfg(test)]
mod cycle12_rust_cross_file_tests;

#[cfg(test)]
mod cycle12_surface_tests;

#[cfg(test)]
mod cycle13_cjs_and_use_path_tests;

#[cfg(test)]
mod cycle13_surface_tests;

#[cfg(test)]
mod cycle14_type_reference_tests;

#[cfg(test)]
mod cycle14_diagnostic_interaction_tests;

#[cfg(test)]
mod cycle14_surface_tests;

#[cfg(test)]
mod cycle15_fp_triage_tests;

#[cfg(test)]
mod cycle15_proximity_tests;

#[cfg(test)]
mod cycle16_method_call_tests;

#[cfg(test)]
mod cycle16_stale_ref_fix_tests;

#[cfg(test)]
mod cycle17_child_module_tests;

#[cfg(test)]
mod cycle18_analysis_tests;

#[cfg(test)]
mod cycle19_analysis_tests;

#[cfg(test)]
mod cycle20_analysis_tests;

#[cfg(test)]
mod cycle20_surface_tests;

#[cfg(test)]
mod cycle21_surface_tests;

#[cfg(test)]
mod cycle21_analysis_tests;

#[cfg(test)]
mod cycle21_qa1_tests;

// Re-export key public types
pub use analyzer::diagnostic::{Confidence, Diagnostic, DiagnosticPattern, Evidence, Severity};
pub use analyzer::flow::{
    trace_all_flows, trace_flows_from, trace_flows_to, FlowPath, FlowStep as FlowPathStep,
};
pub use analyzer::patterns::{run_all_patterns, run_patterns, PatternFilter};
pub use config::Config;
pub use error::{FlowspecError, ManifestError};
pub use graph::Graph;
pub use manifest::types::*;
pub use manifest::{
    validate_manifest_size, JsonFormatter, OutputFormatter, SarifFormatter, SummaryFormatter,
    YamlFormatter,
};

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use analyzer::conversion::to_manifest_entries;
use analyzer::extraction::{
    extract_called_by, extract_calls, extract_dependency_graph, extract_visibility,
    infer_module_role,
};
use graph::populate_graph;
use graph::resolve_cross_file_imports;
use parser::ir::SymbolKind;
use parser::javascript::JsAdapter;
use parser::python::PythonAdapter;
use parser::rust::RustAdapter;
use parser::LanguageAdapter;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// YAML output (default, implemented).
    Yaml,
    /// JSON output.
    Json,
    /// SARIF v2.1.0 output for CI integration (GitHub Code Scanning, Azure DevOps).
    Sarif,
    /// Human-readable summary — compact plain-text output (~2K tokens).
    Summary,
}

/// Supported v1 languages.
const SUPPORTED_LANGUAGES: &[&str] = &["python", "javascript", "typescript", "rust"];

/// Valid diagnostic pattern names.
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

/// Result of running [`analyze()`] on a project.
///
/// Contains both the serializable [`Manifest`] and the live [`Graph`], so
/// callers can run additional queries (e.g. [`trace_flows_from()`]) after
/// analysis without re-parsing.
pub struct AnalysisResult {
    /// The generated manifest — ready for formatting via [`OutputFormatter`].
    pub manifest: Manifest,
    /// Whether any critical-severity diagnostics were found (drives exit code 2).
    pub has_critical: bool,
    /// Whether any findings exist at or above the given thresholds.
    pub has_findings: bool,
    /// The populated analysis graph, available for direct queries
    /// (e.g. [`trace_flows_from()`], [`Graph::callees()`]).
    pub graph: Graph,
    /// Total bytes of source code read during analysis.
    pub source_bytes: u64,
}

/// Run full analysis on a project path and produce a manifest.
///
/// This is the main entry point for the library. It orchestrates the full
/// pipeline: parse → graph → cross-file resolution → analyze → manifest.
///
/// # Parameters
///
/// - `project_path` — root directory (or single file) to analyze. Must exist.
/// - `config` — project configuration loaded from `.flowspec/config.yaml`.
///   Supplies `exclude` patterns for file discovery and `languages` as a
///   fallback when the CLI `--language` flag is not set.
/// - `languages` — restrict analysis to these languages (e.g. `["python"]`).
///   Pass an empty slice to use config languages or auto-detect from extensions.
///
/// # Returns
///
/// An [`AnalysisResult`] containing the generated [`Manifest`], the populated
/// [`Graph`] (available for direct queries like [`trace_flows_from()`]), total
/// source bytes read, and flags indicating whether critical or any diagnostics
/// were found.
///
/// # Errors
///
/// - [`FlowspecError::EmptyPath`] — `project_path` is empty.
/// - [`FlowspecError::TargetNotFound`] — `project_path` does not exist.
/// - [`FlowspecError::UnsupportedLanguage`] — a requested language is not in
///   the supported set (`python`, `javascript`, `typescript`, `rust`).
///
/// # Pipeline stages
///
/// 1. **Discover** source files, skipping generated directories
///    (`target/`, `node_modules/`, `__pycache__/`, etc.), config `exclude`
///    patterns, and `.gitignore`-matched paths.
/// 2. **Parse** each file with the appropriate [`LanguageAdapter`] (Python, JS,
///    Rust) via tree-sitter, producing IR and populating the [`Graph`].
/// 3. **Resolve** cross-file imports by building a module map and linking
///    import symbols to definitions in other files.
/// 4. **Analyze** — run all registered diagnostic patterns on the graph.
/// 5. **Assemble** the [`Manifest`] with entities, flows, diagnostics,
///    modules, and dependency edges.
pub fn analyze(
    project_path: &Path,
    config: &Config,
    languages: &[String],
) -> Result<AnalysisResult, FlowspecError> {
    // Validate path
    if project_path.as_os_str().is_empty() {
        return Err(FlowspecError::EmptyPath);
    }
    if !project_path.exists() {
        return Err(FlowspecError::TargetNotFound {
            path: project_path.to_path_buf(),
        });
    }

    // Validate requested languages
    for lang in languages {
        if !SUPPORTED_LANGUAGES.contains(&lang.as_str()) {
            return Err(FlowspecError::UnsupportedLanguage {
                language: lang.clone(),
            });
        }
    }

    // Discover source files (respects .gitignore, config exclude, hardcoded skip_dirs)
    let (files, detected_langs) = discover_source_files(project_path, &config.exclude);

    // Language priority: CLI flags > config languages > auto-detect
    let active_languages = if !languages.is_empty() {
        languages.to_vec()
    } else if !config.languages.is_empty() {
        config.languages.clone()
    } else {
        detected_langs
    };

    // Build the project name from the directory
    let project_name = project_path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Stage 1: Parse source files and populate the analysis graph
    let all_adapters: Vec<Box<dyn LanguageAdapter>> = vec![
        Box::new(PythonAdapter::new()),
        Box::new(JsAdapter::new()),
        Box::new(RustAdapter::new()),
    ];

    // Filter adapters when languages are specified (CLI flags or config).
    // "typescript" maps to the JS adapter (language_name: "javascript").
    let adapters: Vec<&Box<dyn LanguageAdapter>> = if active_languages.is_empty() {
        all_adapters.iter().collect()
    } else {
        let requested: HashSet<&str> = active_languages.iter().map(|s| s.as_str()).collect();
        all_adapters
            .iter()
            .filter(|a| {
                let name = a.language_name();
                requested.contains(name)
                    || (name == "javascript" && requested.contains("typescript"))
            })
            .collect()
    };

    let mut graph = Graph::new();
    let mut parsed_file_count: u64 = 0;
    let mut source_bytes: u64 = 0;

    for file in &files {
        if let Some(adapter) = adapters.iter().find(|a| a.can_handle(file)) {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read {}: {}", file.display(), e);
                    continue;
                }
            };
            source_bytes += content.len() as u64;
            match adapter.parse_file(file, &content) {
                Ok(result) => {
                    populate_graph(&mut graph, &result);
                    parsed_file_count += 1;
                }
                Err(e) => {
                    tracing::warn!("Failed to parse {}: {}", file.display(), e);
                }
            }
        }
    }

    // Stage 1b: Cross-file import resolution
    // After all files are parsed, resolve import symbols that reference definitions
    // in other files. This creates real edges with actual SymbolIds instead of
    // SymbolId::default() placeholders.
    let module_map = build_module_map(&files);
    if !module_map.is_empty() {
        resolve_cross_file_imports(&mut graph, &module_map);
    }

    // Warn when explicitly requested languages produce no analysis results
    if !languages.is_empty() && parsed_file_count == 0 {
        for lang in languages {
            tracing::warn!(
                "--language {} produced no analysis results. \
                 Verify that a working adapter exists for this language.",
                lang
            );
        }
    }

    // Stage 2: Run diagnostic patterns on the populated graph
    let raw_diagnostics = run_all_patterns(&graph, project_path);
    let diagnostics = to_manifest_entries(&raw_diagnostics);

    // Stage 3: Build entity list from graph symbols
    //
    // Two-pass approach for disambiguation: first collect qualified names
    // to detect collisions, then build entities with directory-prefixed IDs
    // where needed (e.g., parser/utils::helper vs core/utils::helper).

    // Pass 1: Detect ambiguous qualified names
    let mut name_counts: HashMap<String, u32> = HashMap::new();
    let mut sym_parent_dirs: HashMap<String, String> = HashMap::new();
    for (_sym_id, symbol) in graph.all_symbols() {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        *name_counts
            .entry(symbol.qualified_name.clone())
            .or_insert(0) += 1;
        let parent_dir = symbol
            .location
            .file
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        let file_key = format!(
            "{}|{}",
            symbol.qualified_name,
            symbol.location.file.display()
        );
        sym_parent_dirs.insert(file_key, parent_dir);
    }
    let ambiguous_names: HashSet<String> = name_counts
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .map(|(name, _)| name)
        .collect();

    // Pass 2: Build entities with disambiguated IDs
    let mut entities = Vec::new();
    for (sym_id, symbol) in graph.all_symbols() {
        if symbol.kind == SymbolKind::Module {
            continue;
        }
        let rel_path = symbol
            .location
            .file
            .strip_prefix(project_path)
            .unwrap_or(&symbol.location.file);
        let rel_path = if rel_path.as_os_str().is_empty() {
            symbol
                .location
                .file
                .file_name()
                .map(std::path::Path::new)
                .unwrap_or(&symbol.location.file)
        } else {
            rel_path
        };

        // Disambiguate entity ID when qualified_name collides
        let entity_id = if ambiguous_names.contains(&symbol.qualified_name) {
            let file_key = format!(
                "{}|{}",
                symbol.qualified_name,
                symbol.location.file.display()
            );
            let parent_dir = sym_parent_dirs
                .get(&file_key)
                .map(|s| s.as_str())
                .unwrap_or("");
            if parent_dir.is_empty() {
                symbol.qualified_name.clone()
            } else {
                format!("{}/{}", parent_dir, symbol.qualified_name)
            }
        } else {
            symbol.qualified_name.clone()
        };

        let mut called_by = extract_called_by(&graph, sym_id);
        for importer_id in graph.importers(sym_id) {
            if importer_id != parser::ir::SymbolId::default() {
                if let Some(s) = graph.get_symbol(importer_id) {
                    let name = s.qualified_name.clone();
                    if !called_by.contains(&name) {
                        called_by.push(name);
                    }
                }
            }
        }

        entities.push(EntityEntry {
            id: entity_id,
            kind: format_symbol_kind(symbol.kind),
            vis: extract_visibility(symbol),
            sig: symbol.signature.clone().unwrap_or_default(),
            loc: format!("{}:{}", rel_path.display(), symbol.location.line),
            calls: extract_calls(&graph, sym_id),
            called_by,
            annotations: symbol.annotations.clone(),
        });
    }

    // Stage 4: Build module summaries from graph file data
    let mut file_set: HashSet<PathBuf> = HashSet::new();
    for (_, sym) in graph.all_symbols() {
        file_set.insert(sym.location.file.clone());
    }
    let mut modules: Vec<ModuleSummary> = file_set
        .iter()
        .map(|fp| {
            let count = graph
                .symbols_in_file(fp)
                .iter()
                .filter(|&&id| {
                    graph
                        .get_symbol(id)
                        .map(|s| s.kind != SymbolKind::Module)
                        .unwrap_or(false)
                })
                .count() as u64;
            let role = infer_module_role(&graph, fp);
            let name = fp
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            ModuleSummary {
                name,
                entity_count: count,
                role,
            }
        })
        .collect();
    modules.sort_by(|a, b| b.entity_count.cmp(&a.entity_count));

    // Stage 4b: Trace data flows through the call graph
    let flow_paths = trace_all_flows(&graph);
    let flows: Vec<FlowEntry> = flow_paths
        .iter()
        .enumerate()
        .map(|(i, path)| {
            let entry_name = graph
                .get_symbol(path.entry)
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| "unknown".to_string());

            let last_step = path.steps.last();
            let exit_name = last_step
                .and_then(|step| graph.get_symbol(step.symbol))
                .map(|s| s.qualified_name.clone())
                .unwrap_or_else(|| entry_name.clone());

            let steps: Vec<manifest::types::FlowStep> = path
                .steps
                .iter()
                .map(|step| {
                    let entity = graph
                        .get_symbol(step.symbol)
                        .map(|s| s.qualified_name.clone())
                        .unwrap_or_else(|| "unknown".to_string());
                    manifest::types::FlowStep {
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
        .collect();

    // Stage 4c: Deduplicate flows — hash-based dedup on (entry, exit, steps)
    let flows = deduplicate_flows(flows);

    // Stage 5: Assemble manifest
    let diagnostic_count = diagnostics.len() as u64;
    let has_critical = diagnostics.iter().any(|d| d.severity == "critical");
    let has_findings = !diagnostics.is_empty();

    let critical_count = diagnostics
        .iter()
        .filter(|d| d.severity == "critical")
        .count() as u64;
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == "warning")
        .count() as u64;
    let info_count = diagnostics.iter().filter(|d| d.severity == "info").count() as u64;
    let top_issues: Vec<String> = diagnostics
        .iter()
        .take(5)
        .map(|d| format!("{}: {}", d.pattern, d.message))
        .collect();

    let entry_points: Vec<String> = entities
        .iter()
        .filter(|e| e.id.ends_with("::main") || e.id.contains("__main__"))
        .map(|e| e.id.clone())
        .collect();

    let architecture = if modules.is_empty() {
        "Empty project with no analyzable source files.".to_string()
    } else {
        format!(
            "{} project with {} module(s) and {} entities.",
            if active_languages.len() == 1 {
                active_languages[0].clone()
            } else {
                format!("Multi-language ({})", active_languages.join(", "))
            },
            modules.len(),
            entities.len()
        )
    };

    let manifest = Manifest {
        metadata: Metadata {
            project: project_name,
            analyzed_at: chrono::Utc::now().to_rfc3339(),
            flowspec_version: env!("CARGO_PKG_VERSION").to_string(),
            languages: active_languages,
            file_count: parsed_file_count,
            entity_count: entities.len() as u64,
            flow_count: flows.len() as u64,
            diagnostic_count,
            incremental: false,
            files_changed: 0,
        },
        summary: Summary {
            architecture,
            modules,
            entry_points,
            exit_points: Vec::new(),
            key_flows: Vec::new(),
            diagnostic_summary: DiagnosticSummary {
                critical: critical_count,
                warning: warning_count,
                info: info_count,
                top_issues,
            },
        },
        diagnostics,
        entities,
        flows,
        boundaries: Vec::new(),
        dependency_graph: extract_dependency_graph(&graph)
            .into_iter()
            .map(|dep| {
                let project_prefix = project_path.to_string_lossy();
                let strip = |s: &str| -> String {
                    s.strip_prefix(project_prefix.as_ref())
                        .unwrap_or(s)
                        .trim_start_matches('/')
                        .trim_start_matches('\\')
                        .to_string()
                };
                DependencyEdge {
                    from: strip(&dep.from),
                    to: strip(&dep.to),
                    weight: dep.weight as u64,
                    direction: match dep.direction {
                        analyzer::extraction::DependencyDirection::Unidirectional => {
                            "unidirectional".to_string()
                        }
                        analyzer::extraction::DependencyDirection::Bidirectional => {
                            "bidirectional".to_string()
                        }
                    },
                    issues: dep.issues,
                }
            })
            .collect(),
        type_flows: Vec::new(),
    };

    Ok(AnalysisResult {
        manifest,
        has_critical,
        has_findings,
        graph,
        source_bytes,
    })
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

/// Run diagnostics on a project, returning filtered diagnostic entries.
///
/// Runs the full [`analyze()`] pipeline internally, then filters the
/// resulting diagnostics by severity, confidence, and/or pattern name.
///
/// # Parameters
///
/// - `severity_filter` — minimum severity to include (e.g. `Some(Severity::Warning)`
///   drops info-level findings).
/// - `confidence_filter` — minimum confidence to include.
/// - `checks_filter` — restrict to specific pattern names. Returns
///   [`FlowspecError::UnknownPattern`] for invalid names.
///
/// # Returns
///
/// A tuple of `(filtered_diagnostics, has_findings)`. The boolean indicates
/// whether any diagnostics survived filtering.
pub fn diagnose(
    project_path: &Path,
    config: &Config,
    languages: &[String],
    severity_filter: Option<Severity>,
    confidence_filter: Option<Confidence>,
    checks_filter: Option<&[String]>,
) -> Result<(Vec<DiagnosticEntry>, bool), FlowspecError> {
    // Validate check patterns if provided
    if let Some(checks) = checks_filter {
        for pattern in checks {
            if !pattern.is_empty() && !VALID_PATTERNS.contains(&pattern.as_str()) {
                return Err(FlowspecError::UnknownPattern {
                    pattern: pattern.clone(),
                });
            }
        }
    }

    let result = analyze(project_path, config, languages)?;
    let mut diagnostics = result.manifest.diagnostics;

    // Apply severity filter
    if let Some(min_severity) = severity_filter {
        diagnostics.retain(|d| {
            let sev = Severity::from_str_checked(&d.severity).unwrap_or(Severity::Info);
            sev >= min_severity
        });
    }

    // Apply confidence filter
    if let Some(min_confidence) = confidence_filter {
        diagnostics.retain(|d| {
            let conf = Confidence::from_str_checked(&d.confidence).unwrap_or(Confidence::Low);
            conf >= min_confidence
        });
    }

    // Apply checks (pattern name) filter
    if let Some(checks) = checks_filter {
        let non_empty: Vec<&String> = checks.iter().filter(|c| !c.is_empty()).collect();
        if !non_empty.is_empty() {
            diagnostics.retain(|d| non_empty.iter().any(|c| d.pattern == c.as_str()));
        }
    }

    let has_findings = !diagnostics.is_empty();
    Ok((diagnostics, has_findings))
}

/// Discover source files in a project directory, returning file paths and detected languages.
///
/// Respects three exclusion sources (all active simultaneously):
/// 1. **Hardcoded skip_dirs** — safety net for generated/tool directories
/// 2. **Config exclude patterns** — user-specified in `.flowspec/config.yaml`
/// 3. **`.gitignore`** — automatically respected via the `ignore` crate
fn discover_source_files(
    project_path: &Path,
    exclude_patterns: &[String],
) -> (Vec<PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut languages = HashSet::new();

    // Hardcoded safety-net directories — always skipped, even without config
    let skip_dirs: HashSet<&str> = [
        "target",
        "node_modules",
        "__pycache__",
        ".git",
        ".flowspec",
        "build",
        "dist",
        ".venv",
        "venv",
    ]
    .into_iter()
    .collect();

    // Pre-compile config exclude patterns as globs
    let exclude_globs: Vec<glob::Pattern> = exclude_patterns
        .iter()
        .filter_map(|p| {
            let pattern_str = p.trim_end_matches('/');
            glob::Pattern::new(pattern_str)
                .or_else(|_| glob::Pattern::new(&format!("**/{}", pattern_str)))
                .ok()
        })
        .collect();

    // Build walker using `ignore` crate — respects .gitignore automatically
    let mut builder = ignore::WalkBuilder::new(project_path);
    builder
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true);

    for result in builder.build() {
        let entry = match result {
            Ok(e) => e,
            Err(e) => {
                tracing::warn!("Directory walk error: {}", e);
                continue;
            }
        };

        let path = entry.path();

        // Must be a file
        if !entry.file_type().is_some_and(|ft| ft.is_file()) {
            continue;
        }

        // Check if any ancestor directory is in the hardcoded skip list
        let rel_path = path.strip_prefix(project_path).unwrap_or(path);
        let in_skip_dir = rel_path.components().any(|c| {
            if let std::path::Component::Normal(name) = c {
                name.to_str().is_some_and(|n| skip_dirs.contains(n))
            } else {
                false
            }
        });
        if in_skip_dir {
            continue;
        }

        // Check config exclude patterns against relative path components and file name
        let rel_str = rel_path.to_string_lossy();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let matches_exclude = exclude_globs.iter().any(|g| {
            g.matches(&rel_str)
                || g.matches(file_name)
                || rel_path.components().any(|c| {
                    if let std::path::Component::Normal(name) = c {
                        name.to_str().is_some_and(|n| g.matches(n))
                    } else {
                        false
                    }
                })
        });
        if matches_exclude {
            continue;
        }

        // Filter by supported source file extensions
        if let Some(ext) = path.extension() {
            match ext.to_str() {
                Some("py") => {
                    files.push(path.to_path_buf());
                    languages.insert("python".to_string());
                }
                Some("js" | "jsx" | "mjs" | "cjs") => {
                    files.push(path.to_path_buf());
                    languages.insert("javascript".to_string());
                }
                Some("ts" | "tsx") => {
                    files.push(path.to_path_buf());
                    languages.insert("typescript".to_string());
                }
                Some("rs") => {
                    files.push(path.to_path_buf());
                    languages.insert("rust".to_string());
                }
                _ => {}
            }
        }
    }

    let detected: Vec<String> = languages.into_iter().collect();
    (files, detected)
}

/// Maps a [`SymbolKind`] to the abbreviated string used in entity entries.
fn format_symbol_kind(kind: SymbolKind) -> String {
    match kind {
        SymbolKind::Function => "fn",
        SymbolKind::Method => "method",
        SymbolKind::Class => "class",
        SymbolKind::Struct => "struct",
        SymbolKind::Trait => "trait",
        SymbolKind::Interface => "interface",
        SymbolKind::Module => "module",
        SymbolKind::Variable => "var",
        SymbolKind::Constant => "const",
        SymbolKind::Macro => "macro",
        SymbolKind::Enum => "enum",
    }
    .to_string()
}

/// JS/TS file extensions recognized for module mapping.
const JS_EXTENSIONS: &[&str] = &["js", "jsx", "mjs", "cjs", "ts", "tsx"];

/// Builds a mapping from module names to file paths for both Python and JS/TS.
///
/// **Python:** Maps file paths to dotted module names using standard conventions:
/// - `utils.py` → `"utils"`
/// - `my_package/__init__.py` → `"my_package"`
/// - `my_package/core.py` → `"my_package.core"`
///
/// **JavaScript/TypeScript:** Maps file paths to path-based keys (relative to
/// the common prefix, without extension):
/// - `utils.js` → `"utils"`
/// - `src/helpers.js` → `"src/helpers"`
/// - `components/App.tsx` → `"components/App"`
///
/// Both Python and JS/TS entries coexist in the same map. The resolution pass
/// uses the `"from:<module>"` annotation to determine the key format.
pub fn build_module_map(files: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();

    // Phase 1: Python files (dotted module names)
    let py_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "py").unwrap_or(false))
        .collect();

    if !py_files.is_empty() {
        let common_prefix = find_common_prefix(&py_files);
        for file in &py_files {
            let rel = file.strip_prefix(&common_prefix).unwrap_or(file);
            let module_name = path_to_module_name(rel);
            if !module_name.is_empty() {
                map.insert(module_name, (*file).clone());
            }
        }
    }

    // Phase 2: JS/TS files (path-based keys, no extension)
    let js_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| {
            f.extension()
                .and_then(|e| e.to_str())
                .map(|e| JS_EXTENSIONS.contains(&e))
                .unwrap_or(false)
        })
        .collect();

    if !js_files.is_empty() {
        let js_prefix = find_js_common_prefix(&js_files);
        for file in &js_files {
            let rel = file.strip_prefix(&js_prefix).unwrap_or(file);
            let key = js_path_to_module_key(rel);
            if !key.is_empty() {
                map.insert(key, (*file).clone());
            }
        }
    }

    // Phase 3: Rust files (crate-path keys)
    let rs_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "rs").unwrap_or(false))
        .collect();

    if !rs_files.is_empty() {
        let crate_roots = find_rust_crate_roots(&rs_files);
        if crate_roots.is_empty() {
            // No crate root found — use common prefix as fallback
            let common = find_js_common_prefix(&rs_files);
            for file in &rs_files {
                if let Ok(rel) = file.strip_prefix(&common) {
                    let key = rust_path_to_module_key(rel);
                    if !key.is_empty() {
                        map.insert(key, (*file).clone());
                    }
                }
            }
        } else {
            for file in &rs_files {
                if let Some(root_dir) = closest_rust_crate_root(file, &crate_roots) {
                    if let Ok(rel) = file.strip_prefix(root_dir) {
                        let key = rust_path_to_module_key(rel);
                        if !key.is_empty() {
                            map.insert(key, (*file).clone());
                        }
                    }
                }
            }
        }
    }

    map
}

/// Finds directories containing Rust crate roots (`lib.rs` or `main.rs`).
fn find_rust_crate_roots(rs_files: &[&PathBuf]) -> Vec<PathBuf> {
    rs_files
        .iter()
        .filter(|f| {
            f.file_name()
                .map(|n| n == "lib.rs" || n == "main.rs")
                .unwrap_or(false)
        })
        .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
        .collect()
}

/// Finds the closest crate root directory that is an ancestor of the given file.
fn closest_rust_crate_root<'a>(file: &Path, roots: &'a [PathBuf]) -> Option<&'a PathBuf> {
    roots
        .iter()
        .filter(|root| file.starts_with(root))
        .max_by_key(|root| root.components().count())
}

/// Converts a relative Rust file path to a `crate::` prefixed module key.
///
/// - `lib.rs` → `"crate"`, `main.rs` → `"crate"`
/// - `utils.rs` → `"crate::utils"`
/// - `parser/mod.rs` → `"crate::parser"`
/// - `parser/rust.rs` → `"crate::parser::rust"`
fn rust_path_to_module_key(rel_path: &Path) -> String {
    let stem = rel_path.with_extension("");
    let parts: Vec<&str> = stem
        .components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => s.to_str(),
            _ => None,
        })
        .collect();

    if parts.is_empty() {
        return String::new();
    }

    if parts.len() == 1 && (parts[0] == "lib" || parts[0] == "main") {
        return "crate".to_string();
    }

    let mut module_parts: Vec<&str> = parts;
    if module_parts.last() == Some(&"mod") {
        module_parts.pop();
    }

    if module_parts.is_empty() {
        return "crate".to_string();
    }

    format!("crate::{}", module_parts.join("::"))
}

/// Finds the common directory prefix for JS/TS files.
fn find_js_common_prefix(files: &[&PathBuf]) -> PathBuf {
    if files.is_empty() {
        return PathBuf::new();
    }

    let roots: Vec<PathBuf> = files
        .iter()
        .filter_map(|f| f.parent().map(|p| p.to_path_buf()))
        .collect();

    if roots.is_empty() {
        return PathBuf::new();
    }

    let mut prefix = roots[0].clone();
    for root in &roots[1..] {
        while !root.starts_with(&prefix) {
            if !prefix.pop() {
                return PathBuf::new();
            }
        }
    }

    prefix
}

/// Converts a relative JS/TS file path to a module map key.
///
/// Strips the file extension and converts to forward-slash path:
/// - `helpers.js` → `"helpers"`
/// - `src/utils.ts` → `"src/utils"`
/// - `index.js` → `"index"`
fn js_path_to_module_key(rel_path: &Path) -> String {
    let stem = rel_path.with_extension("");
    stem.components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Finds the project root directory for module name computation.
///
/// For `__init__.py` files, uses the grandparent directory (since the parent
/// directory IS part of the package path). For regular `.py` files, uses the
/// parent directory. The result is the common prefix of all these "effective
/// roots", ensuring package structure is preserved in module names.
fn find_common_prefix(files: &[&PathBuf]) -> PathBuf {
    if files.is_empty() {
        return PathBuf::new();
    }

    // For each file, compute its "effective root" — the directory that should
    // NOT be part of the module name.
    let effective_roots: Vec<PathBuf> = files
        .iter()
        .map(|f| {
            let is_init = f
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n == "__init__.py")
                .unwrap_or(false);

            if is_init {
                // __init__.py: grandparent is the project root
                f.parent()
                    .and_then(|p| p.parent())
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| f.parent().map(|p| p.to_path_buf()).unwrap_or_default())
            } else {
                f.parent().map(|p| p.to_path_buf()).unwrap_or_default()
            }
        })
        .collect();

    if effective_roots.is_empty() {
        return PathBuf::new();
    }

    let mut prefix = effective_roots[0].clone();
    for root in &effective_roots[1..] {
        while !root.starts_with(&prefix) {
            if !prefix.pop() {
                return PathBuf::new();
            }
        }
    }

    prefix
}

/// Converts a relative Python file path to a module name.
///
/// - `utils.py` → `"utils"`
/// - `__init__.py` → `""` (maps to parent package, handled by caller)
/// - `pkg/__init__.py` → `"pkg"`
/// - `pkg/core.py` → `"pkg.core"`
fn path_to_module_name(rel_path: &Path) -> String {
    let file_name = rel_path.file_name().and_then(|n| n.to_str()).unwrap_or("");

    if file_name == "__init__.py" {
        if let Some(parent) = rel_path.parent() {
            return components_to_module(parent);
        }
        return String::new();
    }

    let stem = rel_path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
    if stem.is_empty() {
        return String::new();
    }

    if let Some(parent) = rel_path.parent() {
        let parent_module = components_to_module(parent);
        if parent_module.is_empty() {
            stem.to_string()
        } else {
            format!("{}.{}", parent_module, stem)
        }
    } else {
        stem.to_string()
    }
}

/// Converts path components to a dot-separated module name.
fn components_to_module(path: &Path) -> String {
    path.components()
        .filter_map(|c| {
            if let std::path::Component::Normal(s) = c {
                s.to_str()
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}
