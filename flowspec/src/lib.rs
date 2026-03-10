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
//! Source files → Parser (tree-sitter) → IR → Graph → Analyzers → Manifest
//! ```
//!
//! The graph is the source of truth. Manifests are exports optimized
//! for different consumers (YAML for agents, JSON for tools, summary
//! for humans).

pub mod analyzer;
pub mod config;
pub mod error;
pub mod graph;
pub mod manifest;
pub mod parser;

#[cfg(test)]
pub mod test_utils;

// Re-export key public types
pub use analyzer::diagnostic::{
    Confidence as AnalyzerConfidence, Diagnostic, DiagnosticPattern, Evidence,
    Severity as AnalyzerSeverity,
};
pub use analyzer::patterns::{run_all_patterns, run_patterns, PatternFilter};
pub use config::Config;
pub use error::{FlowspecError, ManifestError};
pub use graph::Graph;
pub use manifest::types::*;
pub use manifest::{OutputFormatter, YamlFormatter};

use std::path::Path;

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    /// YAML output (default, implemented).
    Yaml,
    /// JSON output (not yet implemented).
    Json,
    /// SARIF output for CI integration (not yet implemented).
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

/// Severity levels for diagnostic filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// Informational — suboptimal but not broken.
    Info,
    /// Warning — structural defect that will cause problems.
    Warning,
    /// Critical — breaks correctness or causes data loss.
    Critical,
}

/// Confidence levels for diagnostic filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Confidence {
    /// Low confidence — may be a false positive.
    Low,
    /// Moderate confidence — likely a real issue.
    Moderate,
    /// High confidence — structural proof exists.
    High,
}

impl Severity {
    /// Parse a severity string.
    pub fn from_str_checked(s: &str) -> Option<Self> {
        match s {
            "critical" => Some(Severity::Critical),
            "warning" => Some(Severity::Warning),
            "info" => Some(Severity::Info),
            _ => None,
        }
    }
}

impl Confidence {
    /// Parse a confidence string.
    pub fn from_str_checked(s: &str) -> Option<Self> {
        match s {
            "high" => Some(Confidence::High),
            "moderate" => Some(Confidence::Moderate),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
}

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

    // For cycle 1: basic analysis — detect entities from Python files,
    // run basic diagnostics (dead code, phantom dependency)
    let (entities, diagnostics) = analyze_python_files(&files, project_path);

    let diagnostic_count = diagnostics.len() as u64;
    let has_critical = diagnostics.iter().any(|d| d.severity == "critical");
    let has_findings = !diagnostics.is_empty();

    // Build diagnostic summary
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

    // Build summary
    let modules: Vec<ModuleSummary> = group_entities_into_modules(&entities);
    let entry_points: Vec<String> = entities
        .iter()
        .filter(|e| e.id.ends_with("::main") || e.id.contains("__main__"))
        .map(|e| e.id.clone())
        .collect();

    let architecture = if modules.is_empty() {
        "Empty project with no analyzable source files.".to_string()
    } else {
        format!(
            "Python project with {} module(s) and {} entities.",
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
            file_count: files.len() as u64,
            entity_count: entities.len() as u64,
            flow_count: 0,
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
        flows: Vec::new(),
        boundaries: Vec::new(),
        dependency_graph: Vec::new(),
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
                    Some("js" | "jsx") => {
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

/// Basic Python file analysis for cycle 1.
///
/// Scans Python files for function/class definitions and imports,
/// then detects dead code and phantom dependencies.
fn analyze_python_files(
    files: &[std::path::PathBuf],
    project_root: &Path,
) -> (Vec<EntityEntry>, Vec<DiagnosticEntry>) {
    let mut entities = Vec::new();
    let mut all_definitions: Vec<(String, String, String, u32)> = Vec::new(); // (id, kind, file_rel, line)
    let mut all_calls: Vec<String> = Vec::new();
    let mut all_imports: Vec<(String, String, u32)> = Vec::new(); // (import_name, file_rel, line)
    let mut all_used_names: Vec<String> = Vec::new();

    let py_files: Vec<&std::path::PathBuf> = files
        .iter()
        .filter(|f| f.extension().map(|e| e == "py").unwrap_or(false))
        .collect();

    for file in &py_files {
        let content = match std::fs::read_to_string(file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let rel_path = file
            .strip_prefix(project_root)
            .unwrap_or(file)
            .to_string_lossy()
            .to_string();

        let module_name = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        for (line_num, line) in content.lines().enumerate() {
            let trimmed = line.trim();
            let line_number = (line_num + 1) as u32;

            // Detect function definitions
            if trimmed.starts_with("def ") {
                if let Some(name) = extract_function_name(trimmed) {
                    let id = format!("{}::{}", module_name, name);
                    let sig = extract_signature(trimmed);
                    let kind = if is_inside_class(&content, line_num) {
                        "method"
                    } else {
                        "fn"
                    };
                    all_definitions.push((
                        id.clone(),
                        kind.to_string(),
                        rel_path.clone(),
                        line_number,
                    ));
                    entities.push(EntityEntry {
                        id,
                        kind: kind.to_string(),
                        vis: "pub".to_string(),
                        sig,
                        loc: format!("{}:{}", rel_path, line_number),
                        calls: Vec::new(),
                        called_by: Vec::new(),
                        annotations: Vec::new(),
                    });
                }
            }

            // Detect class definitions
            if trimmed.starts_with("class ") {
                if let Some(name) = extract_class_name(trimmed) {
                    let id = format!("{}::{}", module_name, name);
                    all_definitions.push((
                        id.clone(),
                        "class".to_string(),
                        rel_path.clone(),
                        line_number,
                    ));
                    entities.push(EntityEntry {
                        id,
                        kind: "class".to_string(),
                        vis: "pub".to_string(),
                        sig: String::new(),
                        loc: format!("{}:{}", rel_path, line_number),
                        calls: Vec::new(),
                        called_by: Vec::new(),
                        annotations: Vec::new(),
                    });
                }
            }

            // Detect imports
            if trimmed.starts_with("import ") {
                let import_name = trimmed
                    .trim_start_matches("import ")
                    .split(|c: char| c.is_whitespace() || c == ',')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                if !import_name.is_empty() {
                    all_imports.push((import_name, rel_path.clone(), line_number));
                }
            } else if trimmed.starts_with("from ") && trimmed.contains("import") {
                // from X import Y, Z
                if let Some(imports_part) = trimmed.split("import").nth(1) {
                    for name in imports_part.split(',') {
                        let name = name.split_whitespace().next().unwrap_or("").trim();
                        if !name.is_empty() && name != "*" {
                            all_imports.push((name.to_string(), rel_path.clone(), line_number));
                        }
                    }
                }
            }

            // Collect function calls and name references
            // Skip def/class lines for call detection — those are definitions, not calls
            let is_definition = trimmed.starts_with("def ") || trimmed.starts_with("class ");
            for word in trimmed.split(|c: char| !c.is_alphanumeric() && c != '_') {
                if !word.is_empty() && !is_python_keyword(word) {
                    all_used_names.push(word.to_string());
                    // Detect function calls (word followed by `(`)
                    // but not on definition lines where `def name(` is not a call
                    if !is_definition && trimmed.contains(&format!("{}(", word)) {
                        all_calls.push(word.to_string());
                    }
                }
            }
        }
    }

    // Build call relationships
    for entity in &mut entities {
        let name = entity.id.split("::").last().unwrap_or("");
        if all_calls.contains(&name.to_string()) {
            // Mark as called
        }
    }

    // Update called_by relationships
    let call_set: std::collections::HashSet<String> = all_calls.into_iter().collect();
    for entity in &mut entities {
        let name = entity.id.split("::").last().unwrap_or("").to_string();
        if call_set.contains(&name) {
            // This entity is called by something
            entity.called_by = vec!["(detected)".to_string()];
        }
    }

    // Detect diagnostics
    let mut diagnostics = Vec::new();
    let mut diag_id = 1;

    // data_dead_end: functions with zero callers
    for (id, kind, file_rel, line) in &all_definitions {
        if kind == "fn" || kind == "method" {
            let name = id.split("::").last().unwrap_or("");
            // Skip main, __main__, and entry points
            if name == "main" || name.starts_with("__") || name.starts_with("test_") {
                continue;
            }
            if !call_set.contains(name) {
                diagnostics.push(DiagnosticEntry {
                    id: format!("D{:03}", diag_id),
                    pattern: "data_dead_end".to_string(),
                    severity: "warning".to_string(),
                    confidence: "high".to_string(),
                    entity: id.clone(),
                    message: format!("Function {} is never called", name),
                    evidence: format!(
                        "Function `{}` at `{}:{}` has 0 callers across {} analyzed files",
                        name,
                        file_rel,
                        line,
                        py_files.len()
                    ),
                    suggestion: "Remove the function or add a caller. If intentionally unused, mark as entry point.".to_string(),
                    loc: format!("{}:{}", file_rel, line),
                });
                diag_id += 1;
            }
        }
    }

    // phantom_dependency: imports where the imported name is never used
    let used_set: std::collections::HashSet<&str> =
        all_used_names.iter().map(|s| s.as_str()).collect();
    for (import_name, file_rel, line) in &all_imports {
        // Check if the imported name is actually used (beyond the import line itself)
        let base_name = import_name.split('.').next_back().unwrap_or(import_name);
        // Count occurrences — if only 1, it's just the import itself
        let usage_count = all_used_names
            .iter()
            .filter(|n| n.as_str() == base_name)
            .count();
        if usage_count <= 1 && !used_set.contains(base_name) || usage_count == 0 {
            diagnostics.push(DiagnosticEntry {
                id: format!("D{:03}", diag_id),
                pattern: "phantom_dependency".to_string(),
                severity: "info".to_string(),
                confidence: "high".to_string(),
                entity: import_name.clone(),
                message: format!("Import '{}' is never used", import_name),
                evidence: format!(
                    "Import `{}` at `{}:{}` — imported symbol has 0 references in {} analyzed files",
                    import_name, file_rel, line,
                    py_files.len()
                ),
                suggestion: "Remove the unused import to reduce phantom dependencies.".to_string(),
                loc: format!("{}:{}", file_rel, line),
            });
            diag_id += 1;
        }
    }

    (entities, diagnostics)
}

/// Group entities into module summaries.
fn group_entities_into_modules(entities: &[EntityEntry]) -> Vec<ModuleSummary> {
    let mut modules: std::collections::HashMap<String, u64> = std::collections::HashMap::new();
    for entity in entities {
        let module = entity
            .id
            .split("::")
            .next()
            .unwrap_or("unknown")
            .to_string();
        *modules.entry(module).or_insert(0) += 1;
    }
    let mut result: Vec<ModuleSummary> = modules
        .into_iter()
        .map(|(name, count)| ModuleSummary {
            name: name.clone(),
            entity_count: count,
            role: format!("Module with {} entities", count),
        })
        .collect();
    result.sort_by(|a, b| b.entity_count.cmp(&a.entity_count));
    result
}

/// Extract function name from a `def name(...)` line.
fn extract_function_name(line: &str) -> Option<String> {
    let after_def = line.strip_prefix("def ")?;
    let name = after_def.split('(').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Extract class name from a `class Name(...)` or `class Name:` line.
fn extract_class_name(line: &str) -> Option<String> {
    let after_class = line.strip_prefix("class ")?;
    let name = after_class.split(['(', ':']).next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

/// Extract a compact signature from a def line.
fn extract_signature(line: &str) -> String {
    if let Some(start) = line.find('(') {
        if let Some(end) = line.rfind(')') {
            let params = &line[start..=end];
            // Check for return type
            let rest = &line[end + 1..];
            if let Some(arrow_pos) = rest.find("->") {
                let ret_type = rest[arrow_pos + 2..].trim().trim_end_matches(':').trim();
                return format!("{} -> {}", params, ret_type);
            }
            return params.to_string();
        }
    }
    String::new()
}

/// Simple heuristic to detect if a line is inside a class body.
fn is_inside_class(content: &str, target_line: usize) -> bool {
    // Look backwards for a class definition with less indentation
    let lines: Vec<&str> = content.lines().collect();
    if target_line >= lines.len() {
        return false;
    }
    let target_indent = lines[target_line].len() - lines[target_line].trim_start().len();
    if target_indent == 0 {
        return false;
    }

    for i in (0..target_line).rev() {
        let line = lines[i];
        let indent = line.len() - line.trim_start().len();
        if indent < target_indent && line.trim().starts_with("class ") {
            return true;
        }
        if indent == 0 && !line.trim().is_empty() && !line.trim().starts_with('#') {
            break;
        }
    }
    false
}

/// Check if a word is a Python keyword (to avoid counting as a reference).
fn is_python_keyword(word: &str) -> bool {
    matches!(
        word,
        "def"
            | "class"
            | "import"
            | "from"
            | "if"
            | "else"
            | "elif"
            | "for"
            | "while"
            | "return"
            | "yield"
            | "with"
            | "as"
            | "try"
            | "except"
            | "finally"
            | "raise"
            | "pass"
            | "break"
            | "continue"
            | "and"
            | "or"
            | "not"
            | "in"
            | "is"
            | "lambda"
            | "global"
            | "nonlocal"
            | "assert"
            | "del"
            | "True"
            | "False"
            | "None"
            | "async"
            | "await"
            | "print"
            | "self"
    )
}
