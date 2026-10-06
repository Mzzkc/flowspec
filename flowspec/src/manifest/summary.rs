//! Summary output formatter — compact, token-efficient text output.
//!
//! Produces a ~2K token plain-text summary of analysis results.
//! Designed for quick consumption by humans and AI agents who need
//! structural understanding without the full manifest.

use crate::error::ManifestError;
use crate::manifest::{DiagnosticEntry, Manifest, OutputFormatter};

/// Summary formatter for manifest and diagnostic output.
///
/// Produces plain-text output with aligned sections and counts.
/// Target budget: ~2K tokens (~8KB text) for typical projects.
pub struct SummaryFormatter;

impl SummaryFormatter {
    /// Create a new summary formatter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SummaryFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for SummaryFormatter {
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError> {
        let mut lines = Vec::new();

        // Header
        lines.push(format!("Flowspec Analysis: {}", manifest.metadata.project));
        lines.push(format!(
            "Version: {} | Analyzed: {}",
            manifest.metadata.flowspec_version, manifest.metadata.analyzed_at
        ));
        lines.push(String::new());

        // Counts
        lines.push("--- Overview ---".to_string());
        lines.push(format!(
            "Files: {}  Entities: {}  Flows: {}  Diagnostics: {}",
            manifest.metadata.file_count,
            manifest.metadata.entity_count,
            manifest.metadata.flow_count,
            manifest
                .authority
                .as_ref()
                .map_or(manifest.metadata.diagnostic_count, |a| a["findings"]
                    ["total"]
                    .as_u64()
                    .unwrap_or(0))
        ));
        if !manifest.metadata.languages.is_empty() {
            lines.push(format!(
                "Languages: {}",
                manifest.metadata.languages.join(", ")
            ));
        }
        lines.push(String::new());

        // Architecture
        if !manifest.summary.architecture.is_empty() {
            lines.push("--- Architecture ---".to_string());
            lines.push(manifest.summary.architecture.clone());
            lines.push(String::new());
        }

        // Diagnostics summary
        let ds = &manifest.summary.diagnostic_summary;
        if let Some(authority) = &manifest.authority {
            let counts = &authority["findings"]["by_severity"];
            lines.push("--- Diagnostics ---".into());
            lines.push(format!(
                "Critical: {}  Warning: {}  Info: {}",
                counts["critical"].as_u64().unwrap_or(0),
                counts["warning"].as_u64().unwrap_or(0),
                counts["info"].as_u64().unwrap_or(0)
            ));
            let findings = authority["findings"]["all"].as_array().ok_or_else(|| {
                ManifestError::Serialization {
                    reason: "summary requires the complete authority findings".into(),
                }
            })?;
            let indices = authority["display"]["indices"].as_array().ok_or_else(|| {
                ManifestError::Serialization {
                    reason: "summary requires authority display indices".into(),
                }
            })?;
            lines.push(format!(
                "full_findings_total: {}  matched: {}  shown: {}",
                authority["findings"]["total"],
                indices.len(),
                indices.len().min(10)
            ));
            lines.push(format!(
                "gate: {}  qualifying_count: {}",
                authority["gate"]["verdict"].as_str().unwrap_or("unknown"),
                authority["gate"]["qualifying_count"]
            ));
            lines.push(String::new());
            if !indices.is_empty() {
                lines.push("--- Findings ---".into());
                for index in indices.iter().take(10) {
                    let finding = index
                        .as_u64()
                        .and_then(|i| findings.get(i as usize))
                        .ok_or_else(|| ManifestError::Serialization {
                            reason: "invalid authority display index".into(),
                        })?;
                    let range = &finding["range"];
                    lines.push(format!(
                        "  [{}] {} — {} (path_key: {} bytes: {}..{})",
                        finding["severity"].as_str().unwrap_or("").to_uppercase(),
                        finding["pattern_id"].as_str().unwrap_or(""),
                        finding["subject"]["qualified_name"]
                            .as_str()
                            .or_else(|| finding["subject"]["name"].as_str())
                            .unwrap_or("source finding"),
                        range["path_key"].as_str().unwrap_or(""),
                        range["start_byte"],
                        range["end_byte"]
                    ));
                    lines.push(format!(
                        "    semantic_id: {}  observation_id: {}",
                        finding["semantic_id"].as_str().unwrap_or(""),
                        finding["observation_id"].as_str().unwrap_or("")
                    ));
                    if let Some(suggestion) = finding["suggestion"].as_str() {
                        lines.push(format!("    Fix: {suggestion}"));
                    }
                }
                if indices.len() > 10 {
                    lines.push(format!("  ... and {} more findings", indices.len() - 10));
                }
                lines.push(String::new());
            }
        } else if ds.critical > 0 || ds.warning > 0 || ds.info > 0 {
            lines.push("--- Diagnostics ---".to_string());
            lines.push(format!(
                "Critical: {}  Warning: {}  Info: {}",
                ds.critical, ds.warning, ds.info
            ));
            if !ds.top_issues.is_empty() {
                lines.push(String::new());
                lines.push("Top issues:".to_string());
                for (i, issue) in ds.top_issues.iter().take(5).enumerate() {
                    lines.push(format!("  {}. {}", i + 1, issue));
                }
            }
            lines.push(String::new());
        }

        // Modules (sorted by entity_count desc, max 10)
        if !manifest.summary.modules.is_empty() {
            lines.push("--- Modules ---".to_string());
            for module in manifest.summary.modules.iter().take(10) {
                lines.push(format!(
                    "  {} ({} entities) — {}",
                    module.name, module.entity_count, module.role
                ));
            }
            if manifest.summary.modules.len() > 10 {
                lines.push(format!(
                    "  ... and {} more",
                    manifest.summary.modules.len() - 10
                ));
            }
            lines.push(String::new());
        }

        // Entry points
        if !manifest.summary.entry_points.is_empty() {
            lines.push("--- Entry Points ---".to_string());
            for ep in &manifest.summary.entry_points {
                lines.push(format!("  {}", ep));
            }
            lines.push(String::new());
        }

        // Findings list (max 10)
        if manifest.authority.is_none() && !manifest.diagnostics.is_empty() {
            lines.push("--- Findings ---".to_string());
            for diag in manifest.diagnostics.iter().take(10) {
                lines.push(format!(
                    "  [{}] {} — {} ({})",
                    diag.severity.to_uppercase(),
                    diag.pattern,
                    diag.message,
                    diag.loc
                ));
            }
            if manifest.diagnostics.len() > 10 {
                lines.push(format!(
                    "  ... and {} more findings",
                    manifest.diagnostics.len() - 10
                ));
            }
            lines.push(String::new());
        }

        Ok(lines.join("\n"))
    }

    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError> {
        let mut lines = Vec::new();

        lines.push(format!("Diagnostics: {} finding(s)", diagnostics.len()));
        lines.push(String::new());

        if diagnostics.is_empty() {
            lines.push("No findings.".to_string());
            return Ok(lines.join("\n"));
        }

        // Count by severity
        let critical = diagnostics
            .iter()
            .filter(|d| d.severity == "critical")
            .count();
        let warning = diagnostics
            .iter()
            .filter(|d| d.severity == "warning")
            .count();
        let info = diagnostics.iter().filter(|d| d.severity == "info").count();

        lines.push(format!(
            "Critical: {}  Warning: {}  Info: {}",
            critical, warning, info
        ));
        lines.push(String::new());

        for diag in diagnostics {
            lines.push(format!(
                "[{}] {} — {} ({})",
                diag.severity.to_uppercase(),
                diag.pattern,
                diag.message,
                diag.loc
            ));
            if !diag.suggestion.is_empty() {
                lines.push(format!("  Fix: {}", diag.suggestion));
            }
        }

        Ok(lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_formatter_produces_output_for_full_manifest() {
        let manifest = Manifest::sample_full();
        let formatter = SummaryFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(result.is_ok(), "Summary format failed: {:?}", result.err());

        let output = result.unwrap();
        assert!(!output.is_empty());
        assert!(output.contains("Flowspec Analysis:"));
        assert!(output.contains("test-project"));
    }

    #[test]
    fn summary_formatter_produces_output_for_empty_manifest() {
        let manifest = Manifest::empty();
        let formatter = SummaryFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(
            result.is_ok(),
            "Empty manifest summary failed: {:?}",
            result.err()
        );

        let output = result.unwrap();
        assert!(!output.is_empty());
        // Must have overview section even for empty project
        assert!(output.contains("Files: 0"));
    }

    #[test]
    fn summary_output_size_bounded() {
        let manifest = Manifest::sample_full();
        let formatter = SummaryFormatter::new();
        let output = formatter.format_manifest(&manifest).unwrap();

        // Must be bounded (~2K tokens ≈ ~8KB). Allow generous 16KB.
        assert!(
            output.len() < 16_384,
            "Summary output too large: {} bytes",
            output.len()
        );
        assert!(
            output.len() > 50,
            "Summary output suspiciously small: {} bytes",
            output.len()
        );
    }

    #[test]
    fn summary_contains_key_sections() {
        let manifest = Manifest::sample_full();
        let formatter = SummaryFormatter::new();
        let output = formatter.format_manifest(&manifest).unwrap();

        assert!(
            output.contains("File") || output.contains("file"),
            "Summary must mention files"
        );
        assert!(
            output.contains("Entit") || output.contains("entit"),
            "Summary must mention entities"
        );
        assert!(
            output.contains("Diagnostic")
                || output.contains("diagnostic")
                || output.contains("Finding")
                || output.contains("finding"),
            "Summary must mention diagnostics/findings"
        );
    }

    #[test]
    fn summary_format_diagnostics_works() {
        let diagnostics = vec![
            DiagnosticEntry::sample_critical(),
            DiagnosticEntry::sample_warning(),
        ];
        let formatter = SummaryFormatter::new();
        let result = formatter.format_diagnostics(&diagnostics);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("2 finding(s)"));
        assert!(output.contains("Critical: 1"));
    }

    #[test]
    fn summary_format_empty_diagnostics() {
        let diagnostics: Vec<DiagnosticEntry> = vec![];
        let formatter = SummaryFormatter::new();
        let result = formatter.format_diagnostics(&diagnostics);
        assert!(result.is_ok());

        let output = result.unwrap();
        assert!(output.contains("0 finding(s)"));
        assert!(output.contains("No findings"));
    }
}

#[cfg(test)]
mod authority_summary_tests {
    use super::*;

    #[test]
    fn summary_uses_complete_authority_catalog_and_unfiltered_gate() {
        let input = tempfile::tempdir().unwrap();
        std::fs::write(
            input.path().join("unit.py"),
            "def _hidden(value):\n    return value + 1\n",
        )
        .unwrap();
        let report = crate::diagnose_report(
            input.path(),
            &crate::Config::default(),
            &["python".into()],
            &crate::ReportOptions::default(),
        )
        .unwrap();
        let authority = report.authority.as_ref().unwrap();
        assert_eq!(authority["findings"]["total"], 2);
        let before = authority.clone();
        let text = SummaryFormatter::new().format_manifest(&report).unwrap();
        assert!(text.contains("Diagnostics: 2"), "{text}");
        assert!(text.contains("Warning: 2"), "{text}");
        assert!(text.contains("isolated_cluster"), "{text}");
        assert!(text.contains("data_dead_end"), "{text}");
        assert!(text.contains("gate: pass"), "{text}");
        for finding in authority["findings"]["all"].as_array().unwrap() {
            assert!(
                text.contains(finding["semantic_id"].as_str().unwrap()),
                "{text}"
            );
        }
        assert_eq!(report.authority.as_ref().unwrap(), &before);

        let filtered = crate::diagnose_report(
            input.path(),
            &crate::Config::default(),
            &["python".into()],
            &crate::ReportOptions {
                severity: Some(crate::Severity::Critical),
                gate_severity: crate::Severity::Warning,
                ..Default::default()
            },
        )
        .unwrap();
        let text = SummaryFormatter::new().format_manifest(&filtered).unwrap();
        assert!(text.contains("Diagnostics: 2"), "{text}");
        assert!(text.contains("matched: 0"), "{text}");
        assert!(text.contains("gate: fail"), "{text}");
        assert!(!text.contains("[WARNING]"), "{text}");
        assert_eq!(crate::report_exit_code(&filtered, None), 2);
    }
}
