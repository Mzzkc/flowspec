//! Extracted CLI command logic — testable library functions.
//!
//! These functions contain the core logic previously embedded in the CLI binary's
//! `main.rs`. By living in the library crate, they can be unit-tested directly,
//! improving coverage and catching regressions without integration test overhead.

use std::io::Write;
use std::path::{Path, PathBuf};

use crate::error::{FlowspecError, ManifestError};
use crate::manifest::types::FlowEntry;
use crate::manifest::{validate_manifest_size, OutputFormatter};
use crate::{Config, JsonFormatter, OutputFormat, SarifFormatter, SummaryFormatter, YamlFormatter};

/// Trace direction for flow tracing commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraceDirection {
    /// Trace callees (forward data flow).
    Forward,
    /// Trace callers (backward data flow — not yet implemented).
    Backward,
    /// Trace both directions (not yet implemented).
    Both,
}

/// Run the `analyze` command: parse, build graph, produce manifest.
///
/// Returns `(output_string, exit_code)` on success.
pub fn run_analyze(
    path: &Path,
    languages: &[String],
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

    let normalized = normalize_languages(languages);

    tracing::info!("Analyzing project at {}", canonical.display());

    let result = crate::analyze(&canonical, &config, &normalized)?;

    let output = format_with(format, |f| f.format_manifest(&result.manifest))?;

    validate_manifest_size(&output, result.source_bytes)?;

    write_output(&output, output_path)?;

    if result.has_critical {
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
    // Guard unsupported formats (summary handled below per-format)
    if !matches!(
        format,
        OutputFormat::Yaml | OutputFormat::Json | OutputFormat::Sarif | OutputFormat::Summary
    ) {
        return Err(FlowspecError::FormatNotImplemented {
            format: format_name(format).to_string(),
        });
    }

    // Guard unsupported directions
    match direction {
        TraceDirection::Forward => {}
        TraceDirection::Backward => {
            return Err(FlowspecError::CommandNotImplemented {
                command: "--direction backward".to_string(),
                suggestion:
                    "use --direction forward; backward tracing is planned for a future release"
                        .to_string(),
            });
        }
        TraceDirection::Both => {
            return Err(FlowspecError::CommandNotImplemented {
                command: "--direction both".to_string(),
                suggestion:
                    "use --direction forward; bidirectional tracing is planned for a future release"
                        .to_string(),
            });
        }
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

    // Filter flows to only those relevant to the matched symbol
    let mut flow_entries: Vec<FlowEntry> = result
        .manifest
        .flows
        .into_iter()
        .filter(|flow| {
            flow.entry == matched_entity
                || flow.exit == matched_entity
                || flow.steps.iter().any(|s| s.entity == matched_entity)
        })
        .collect();

    // Apply depth truncation
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
            // Summary for trace: render a compact text representation of matching flows
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
        let options: Vec<&str> = name_matches.iter().map(|e| e.id.as_str()).collect();
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
        let options: Vec<&str> = substring_matches.iter().map(|e| e.id.as_str()).collect();
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
}
