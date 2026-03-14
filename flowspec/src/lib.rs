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

pub mod analyzer;
pub mod config;
pub mod error;
pub mod graph;
pub mod manifest;
pub mod parser;

#[cfg(test)]
pub mod test_utils;

#[cfg(test)]
mod pipeline_tests;

#[cfg(test)]
mod pattern_integration_tests;

// Re-export key public types
pub use analyzer::diagnostic::{Confidence, Diagnostic, DiagnosticPattern, Evidence, Severity};
pub use analyzer::flow::{trace_all_flows, trace_flows_from, FlowPath, FlowStep as FlowPathStep};
pub use analyzer::patterns::{run_all_patterns, run_patterns, PatternFilter};
pub use config::Config;
pub use error::{FlowspecError, ManifestError};
pub use graph::Graph;
pub use manifest::types::*;
pub use manifest::{JsonFormatter, OutputFormatter, SarifFormatter, YamlFormatter};

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
    /// Human-readable summary (not yet implemented).
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

/// Result of running `flowspec analyze` on a project.
pub struct AnalysisResult {
    /// The generated manifest.
    pub manifest: Manifest,
    /// Whether any critical diagnostics were found.
    pub has_critical: bool,
    /// Whether any findings exist at or above the given thresholds.
    pub has_findings: bool,
}

/// Run full analysis on a project path and produce a manifest.
///
/// This is the main entry point for the library. It orchestrates:
/// parse → graph → analyze → manifest generation.
pub fn analyze(
    project_path: &Path,
    _config: &Config,
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

    // Discover source files
    let (files, detected_langs) = discover_source_files(project_path);

    // Determine which languages to analyze
    let active_languages = if languages.is_empty() {
        detected_langs
    } else {
        languages.to_vec()
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

    // Filter adapters when --language is specified.
    // "typescript" maps to the JS adapter (language_name: "javascript").
    let adapters: Vec<&Box<dyn LanguageAdapter>> = if languages.is_empty() {
        all_adapters.iter().collect()
    } else {
        let requested: HashSet<&str> = languages.iter().map(|s| s.as_str()).collect();
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

    for file in &files {
        if let Some(adapter) = adapters.iter().find(|a| a.can_handle(file)) {
            let content = match std::fs::read_to_string(file) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!("Failed to read {}: {}", file.display(), e);
                    continue;
                }
            };
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
    let mut entities = Vec::new();
    for (sym_id, symbol) in graph.all_symbols() {
        if symbol.kind == SymbolKind::Module {
            continue; // Skip file-scope module symbols
        }
        let rel_path = symbol
            .location
            .file
            .strip_prefix(project_path)
            .unwrap_or(&symbol.location.file);
        // When analyzing a single file, strip_prefix removes the entire path,
        // producing an empty string. Fall back to the filename component.
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
        // Combine callers (EdgeKind::Calls) and importers (EdgeKind::References)
        // to surface cross-file dependents in the called_by field.
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
            id: symbol.qualified_name.clone(),
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
            .map(|dep| DependencyEdge {
                from: dep.from,
                to: dep.to,
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
            })
            .collect(),
        type_flows: Vec::new(),
    };

    Ok(AnalysisResult {
        manifest,
        has_critical,
        has_findings,
    })
}

/// Run diagnostics on a project, returning only filtered diagnostic entries.
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
fn discover_source_files(project_path: &Path) -> (Vec<std::path::PathBuf>, Vec<String>) {
    let mut files = Vec::new();
    let mut languages = std::collections::HashSet::new();

    // Directories to skip
    let skip_dirs = [
        "target",
        "node_modules",
        "__pycache__",
        ".git",
        ".flowspec",
        "build",
        "dist",
        ".venv",
        "venv",
    ];

    if let Ok(entries) = walk_dir(project_path, &skip_dirs) {
        for entry in entries {
            if let Some(ext) = entry.extension() {
                match ext.to_str() {
                    Some("py") => {
                        files.push(entry);
                        languages.insert("python".to_string());
                    }
                    Some("js" | "jsx" | "mjs") => {
                        files.push(entry);
                        languages.insert("javascript".to_string());
                    }
                    Some("ts" | "tsx") => {
                        files.push(entry);
                        languages.insert("typescript".to_string());
                    }
                    Some("rs") => {
                        files.push(entry);
                        languages.insert("rust".to_string());
                    }
                    _ => {}
                }
            }
        }
    }

    let detected: Vec<String> = languages.into_iter().collect();
    (files, detected)
}

/// Recursively walk a directory, skipping excluded directories.
fn walk_dir(path: &Path, skip_dirs: &[&str]) -> Result<Vec<std::path::PathBuf>, std::io::Error> {
    let mut result = Vec::new();

    if path.is_file() {
        result.push(path.to_path_buf());
        return Ok(result);
    }

    if !path.is_dir() {
        return Ok(result);
    }

    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let entry_path = entry.path();

        if entry_path.is_dir() {
            if let Some(name) = entry_path.file_name().and_then(|n| n.to_str()) {
                if skip_dirs.contains(&name) {
                    continue;
                }
            }
            result.extend(walk_dir(&entry_path, skip_dirs)?);
        } else {
            result.push(entry_path);
        }
    }

    Ok(result)
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

/// Builds a mapping from Python module names to file paths.
///
/// Maps file paths to their Python module names using standard Python conventions:
/// - `utils.py` → `"utils"`
/// - `my_package/__init__.py` → `"my_package"`
/// - `my_package/core.py` → `"my_package.core"`
///
/// Only Python files (`.py` extension) are included. The mapping uses paths
/// relative to the discovered project structure, with path separators converted
/// to dots.
pub fn build_module_map(files: &[PathBuf]) -> HashMap<String, PathBuf> {
    let mut map = HashMap::new();

    let py_files: Vec<&PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "py").unwrap_or(false))
        .collect();

    if py_files.is_empty() {
        return map;
    }

    // Find the common prefix (project root) for all Python files
    let common_prefix = find_common_prefix(&py_files);

    for file in &py_files {
        let rel = file.strip_prefix(&common_prefix).unwrap_or(file);
        let module_name = path_to_module_name(rel);
        if !module_name.is_empty() {
            map.insert(module_name, (*file).clone());
        }
    }

    map
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
