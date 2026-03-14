//! SARIF v2.1.0 output formatter — produces valid SARIF JSON for GitHub Code Scanning.
//!
//! SARIF (Static Analysis Results Interchange Format) is the standard format
//! for static analysis tools integrating with GitHub Code Scanning and other
//! CI systems. This formatter maps Flowspec diagnostics to SARIF results with
//! rule definitions, severity levels, and physical locations.

use serde::Serialize;

use crate::error::ManifestError;
use crate::manifest::{DiagnosticEntry, Manifest, OutputFormatter};

/// SARIF schema URL for v2.1.0.
const SARIF_SCHEMA: &str =
    "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/main/sarif-2.1/schema/sarif-schema-2.1.0.json";

/// SARIF version string.
const SARIF_VERSION: &str = "2.1.0";

// --- SARIF schema structs ---

/// Top-level SARIF log object.
#[derive(Debug, Serialize)]
struct SarifLog {
    #[serde(rename = "$schema")]
    schema: String,
    version: String,
    runs: Vec<Run>,
}

/// A single analysis run.
#[derive(Debug, Serialize)]
struct Run {
    tool: Tool,
    results: Vec<SarifResult>,
}

/// Tool metadata.
#[derive(Debug, Serialize)]
struct Tool {
    driver: ToolDriver,
}

/// Tool driver (the analysis tool itself).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ToolDriver {
    name: String,
    version: String,
    information_uri: String,
    rules: Vec<Rule>,
}

/// A rule definition (maps to a diagnostic pattern).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Rule {
    id: String,
    short_description: Message,
}

/// A SARIF result (maps to a single diagnostic finding).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SarifResult {
    rule_id: String,
    level: String,
    message: Message,
    locations: Vec<Location>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    related_locations: Vec<RelatedLocation>,
}

/// A message object with text content.
#[derive(Debug, Serialize)]
struct Message {
    text: String,
}

/// A physical location in the source code.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Location {
    physical_location: PhysicalLocation,
}

/// Physical location details (file + region).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PhysicalLocation {
    artifact_location: ArtifactLocation,
    #[serde(skip_serializing_if = "Option::is_none")]
    region: Option<Region>,
}

/// File path as a URI.
#[derive(Debug, Serialize)]
struct ArtifactLocation {
    uri: String,
}

/// Source region (line number).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct Region {
    start_line: u64,
}

/// A related location (maps to evidence entries with locations).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelatedLocation {
    id: usize,
    physical_location: PhysicalLocation,
    message: Message,
}

// --- Helpers ---

/// Map Flowspec severity to SARIF level.
///
/// - critical → error
/// - warning → warning
/// - info → note
/// - unknown → warning (safe default)
pub fn severity_to_level(severity: &str) -> &'static str {
    match severity {
        "critical" => "error",
        "warning" => "warning",
        "info" => "note",
        _ => "warning",
    }
}

/// Parse a location string "file:line" into (file, optional line number).
///
/// Splits on the last `:` to handle paths that may contain colons.
/// Returns the full string as the file if no valid line number is found.
pub fn parse_location(loc: &str) -> (&str, Option<u64>) {
    if loc.is_empty() {
        return ("", None);
    }
    if let Some(colon_pos) = loc.rfind(':') {
        let (file_part, line_part) = loc.split_at(colon_pos);
        // line_part starts with ':', skip it
        if let Ok(line) = line_part[1..].parse::<u64>() {
            return (file_part, Some(line));
        }
    }
    (loc, None)
}

/// Build deduplicated rule definitions from diagnostic entries.
///
/// Each unique `pattern` in the diagnostics becomes one rule entry.
fn build_rules(diagnostics: &[DiagnosticEntry]) -> Vec<Rule> {
    let mut seen = std::collections::HashSet::new();
    let mut rules = Vec::new();
    for diag in diagnostics {
        if seen.insert(diag.pattern.clone()) {
            rules.push(Rule {
                id: diag.pattern.clone(),
                short_description: Message {
                    text: diag.pattern.replace('_', " "),
                },
            });
        }
    }
    rules
}

/// Convert a single diagnostic entry to a SARIF result.
fn diagnostic_to_result(diag: &DiagnosticEntry) -> SarifResult {
    let (file, line) = parse_location(&diag.loc);
    let location = Location {
        physical_location: PhysicalLocation {
            artifact_location: ArtifactLocation {
                uri: file.to_string(),
            },
            region: line.map(|l| Region { start_line: l }),
        },
    };

    // Map evidence entries with locations to relatedLocations
    let related_locations: Vec<RelatedLocation> = diag
        .evidence
        .iter()
        .enumerate()
        .filter_map(|(i, ev)| {
            ev.location.as_ref().map(|loc_str| {
                let (ev_file, ev_line) = parse_location(loc_str);
                RelatedLocation {
                    id: i + 1,
                    physical_location: PhysicalLocation {
                        artifact_location: ArtifactLocation {
                            uri: ev_file.to_string(),
                        },
                        region: ev_line.map(|l| Region { start_line: l }),
                    },
                    message: Message {
                        text: ev.observation.clone(),
                    },
                }
            })
        })
        .collect();

    SarifResult {
        rule_id: diag.pattern.clone(),
        level: severity_to_level(&diag.severity).to_string(),
        message: Message {
            text: diag.message.clone(),
        },
        locations: vec![location],
        related_locations,
    }
}

/// Build a complete SARIF log from a slice of diagnostics.
fn build_sarif_log(diagnostics: &[DiagnosticEntry]) -> SarifLog {
    let rules = build_rules(diagnostics);
    let results: Vec<SarifResult> = diagnostics.iter().map(diagnostic_to_result).collect();

    SarifLog {
        schema: SARIF_SCHEMA.to_string(),
        version: SARIF_VERSION.to_string(),
        runs: vec![Run {
            tool: Tool {
                driver: ToolDriver {
                    name: "Flowspec".to_string(),
                    version: env!("CARGO_PKG_VERSION").to_string(),
                    information_uri: "https://github.com/Mzzkc/flowspec".to_string(),
                    rules,
                },
            },
            results,
        }],
    }
}

// --- Formatter ---

/// SARIF v2.1.0 formatter for manifest and diagnostic output.
///
/// Produces JSON conforming to the SARIF v2.1.0 specification, suitable for
/// upload to GitHub Code Scanning, Azure DevOps, and other SARIF consumers.
/// Maps each Flowspec diagnostic to a SARIF result with rule definition,
/// severity level, physical location, and related locations from evidence.
pub struct SarifFormatter;

impl SarifFormatter {
    /// Create a new SARIF formatter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for SarifFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for SarifFormatter {
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError> {
        let sarif = build_sarif_log(&manifest.diagnostics);
        serde_json::to_string_pretty(&sarif).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }

    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError> {
        let sarif = build_sarif_log(diagnostics);
        serde_json::to_string_pretty(&sarif).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::types::{DiagnosticEntry, EvidenceEntry, Manifest};

    fn create_test_diagnostic() -> DiagnosticEntry {
        DiagnosticEntry {
            id: "D001".to_string(),
            pattern: "data_dead_end".to_string(),
            severity: "critical".to_string(),
            confidence: "high".to_string(),
            entity: "module::dead_fn".to_string(),
            message: "Function dead_fn is never called".to_string(),
            evidence: vec![EvidenceEntry {
                observation: "0 callers in 5 files".to_string(),
                location: Some("src/utils.py:42".to_string()),
                context: Some("private function".to_string()),
            }],
            suggestion: "Remove or add a caller".to_string(),
            loc: "src/utils.py:42".to_string(),
        }
    }

    fn create_test_warning() -> DiagnosticEntry {
        DiagnosticEntry {
            id: "D002".to_string(),
            pattern: "phantom_dependency".to_string(),
            severity: "warning".to_string(),
            confidence: "moderate".to_string(),
            entity: "module::import_os".to_string(),
            message: "Import os is unused".to_string(),
            evidence: vec![EvidenceEntry {
                observation: "imported but never referenced".to_string(),
                location: None,
                context: None,
            }],
            suggestion: "Remove unused import".to_string(),
            loc: "main.py:1".to_string(),
        }
    }

    fn create_test_info() -> DiagnosticEntry {
        DiagnosticEntry {
            id: "D003".to_string(),
            pattern: "orphaned_impl".to_string(),
            severity: "info".to_string(),
            confidence: "low".to_string(),
            entity: "module::helper".to_string(),
            message: "Helper has no callers".to_string(),
            evidence: vec![],
            suggestion: "Consider removing".to_string(),
            loc: "helpers.py:10".to_string(),
        }
    }

    // --- severity_to_level tests ---

    #[test]
    fn severity_critical_to_error() {
        assert_eq!(severity_to_level("critical"), "error");
    }

    #[test]
    fn severity_warning_to_warning() {
        assert_eq!(severity_to_level("warning"), "warning");
    }

    #[test]
    fn severity_info_to_note() {
        assert_eq!(severity_to_level("info"), "note");
    }

    #[test]
    fn severity_unknown_defaults_to_warning() {
        assert_eq!(severity_to_level("unknown"), "warning");
    }

    // --- parse_location tests ---

    #[test]
    fn parse_location_file_and_line() {
        let (file, line) = parse_location("example.py:42");
        assert_eq!(file, "example.py");
        assert_eq!(line, Some(42));
    }

    #[test]
    fn parse_location_file_only() {
        let (file, line) = parse_location("example.py");
        assert_eq!(file, "example.py");
        assert_eq!(line, None);
    }

    #[test]
    fn parse_location_empty_string() {
        let (file, line) = parse_location("");
        assert_eq!(file, "");
        assert_eq!(line, None);
    }

    #[test]
    fn parse_location_path_with_directories() {
        let (file, line) = parse_location("src/utils/helpers.py:100");
        assert_eq!(file, "src/utils/helpers.py");
        assert_eq!(line, Some(100));
    }

    #[test]
    fn parse_location_line_zero() {
        let (file, line) = parse_location("file.py:0");
        assert_eq!(file, "file.py");
        assert_eq!(line, Some(0));
    }

    // --- build_rules tests ---

    #[test]
    fn build_rules_deduplicates() {
        let diagnostics = vec![
            create_test_diagnostic(),
            DiagnosticEntry {
                id: "D004".to_string(),
                pattern: "data_dead_end".to_string(),
                severity: "warning".to_string(),
                confidence: "high".to_string(),
                entity: "module::another_fn".to_string(),
                message: "Another dead end".to_string(),
                evidence: vec![],
                suggestion: "Fix it".to_string(),
                loc: "other.py:5".to_string(),
            },
            create_test_warning(),
        ];
        let rules = build_rules(&diagnostics);
        assert_eq!(
            rules.len(),
            2,
            "3 diagnostics with 2 unique patterns should produce 2 rules"
        );
    }

    // --- format_manifest tests ---

    #[test]
    fn format_manifest_valid_sarif() {
        let formatter = SarifFormatter::new();
        let mut manifest = Manifest::sample_full();
        manifest.diagnostics = vec![create_test_diagnostic()];
        let output = formatter.format_manifest(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed["version"], "2.1.0");
        assert!(parsed["$schema"].as_str().unwrap().contains("sarif"));
        assert!(parsed["runs"].is_array());
        assert!(parsed["runs"][0]["results"].is_array());
    }

    // --- format_diagnostics tests ---

    #[test]
    fn format_diagnostics_valid_sarif() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![create_test_diagnostic()];
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed["version"], "2.1.0");
        assert!(parsed["runs"][0]["results"].is_array());
        assert_eq!(parsed["runs"][0]["results"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn format_empty_diagnostics_valid_sarif() {
        let formatter = SarifFormatter::new();
        let output = formatter.format_diagnostics(&[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        assert_eq!(parsed["runs"][0]["results"].as_array().unwrap().len(), 0);
        assert!(parsed["runs"][0]["tool"]["driver"]["name"].is_string());
        assert_eq!(
            parsed["runs"][0]["tool"]["driver"]["name"]
                .as_str()
                .unwrap(),
            "Flowspec"
        );
    }

    #[test]
    fn sarif_severity_mapping_in_results() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![
            create_test_diagnostic(), // critical
            create_test_warning(),    // warning
            create_test_info(),       // info
        ];
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let results = parsed["runs"][0]["results"].as_array().unwrap();

        assert_eq!(results[0]["level"], "error");
        assert_eq!(results[1]["level"], "warning");
        assert_eq!(results[2]["level"], "note");
    }

    #[test]
    fn sarif_location_structure() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![create_test_diagnostic()];
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let result = &parsed["runs"][0]["results"][0];
        let loc = &result["locations"][0]["physicalLocation"];

        assert_eq!(loc["artifactLocation"]["uri"], "src/utils.py");
        assert_eq!(loc["region"]["startLine"], 42);
    }

    #[test]
    fn sarif_rules_match_result_rule_ids() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![create_test_diagnostic(), create_test_warning()];
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        let rules = parsed["runs"][0]["tool"]["driver"]["rules"]
            .as_array()
            .unwrap();
        let results = parsed["runs"][0]["results"].as_array().unwrap();

        let rule_ids: std::collections::HashSet<&str> =
            rules.iter().map(|r| r["id"].as_str().unwrap()).collect();

        for result in results {
            let rule_id = result["ruleId"].as_str().unwrap();
            assert!(
                rule_ids.contains(rule_id),
                "Result ruleId '{}' has no matching rule definition",
                rule_id
            );
        }
    }

    #[test]
    fn sarif_related_locations_from_evidence() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![create_test_diagnostic()];
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let result = &parsed["runs"][0]["results"][0];

        let related = result["relatedLocations"].as_array().unwrap();
        assert_eq!(related.len(), 1);
        assert_eq!(
            related[0]["physicalLocation"]["artifactLocation"]["uri"],
            "src/utils.py"
        );
    }

    #[test]
    fn sarif_no_related_locations_when_no_evidence_location() {
        let formatter = SarifFormatter::new();
        let diagnostics = vec![create_test_warning()]; // evidence has no location
        let output = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();
        let result = &parsed["runs"][0]["results"][0];

        // relatedLocations should be omitted (skip_serializing_if empty)
        assert!(
            result.get("relatedLocations").is_none(),
            "relatedLocations should be omitted when empty"
        );
    }

    #[test]
    fn sarif_tool_version_matches_crate() {
        let formatter = SarifFormatter::new();
        let output = formatter.format_diagnostics(&[]).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&output).unwrap();

        let version = parsed["runs"][0]["tool"]["driver"]["version"]
            .as_str()
            .unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }
}
