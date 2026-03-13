// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Diagnostic → DiagnosticEntry conversion.
//!
//! Bridges the analyzer's rich `Diagnostic` type to the manifest's
//! `DiagnosticEntry` type for output formatting. Every field is mapped
//! with no information loss.

use crate::analyzer::diagnostic::{Diagnostic, Evidence};
use crate::manifest::types::{DiagnosticEntry, EvidenceEntry};

/// Converts a single `Diagnostic` to a manifest-ready `DiagnosticEntry`.
///
/// Field mapping:
/// - `pattern` → `DiagnosticPattern.name()` (snake_case)
/// - `severity` → lowercase Display string
/// - `confidence` → lowercase Display string
/// - `evidence` → formatted multi-observation string (structured when Worker 1 delivers `Vec<EvidenceEntry>`)
/// - `location` → `loc` (field rename)
pub fn to_manifest_entry(diagnostic: &Diagnostic) -> DiagnosticEntry {
    DiagnosticEntry {
        id: diagnostic.id.clone(),
        pattern: diagnostic.pattern.name().to_string(),
        severity: diagnostic.severity.to_string(),
        confidence: diagnostic.confidence.to_string(),
        entity: diagnostic.entity.clone(),
        message: diagnostic.message.clone(),
        evidence: convert_evidence(&diagnostic.evidence),
        suggestion: diagnostic.suggestion.clone(),
        loc: diagnostic.location.clone(),
    }
}

/// Converts a slice of `Diagnostic`s to manifest-ready `DiagnosticEntry`s.
pub fn to_manifest_entries(diagnostics: &[Diagnostic]) -> Vec<DiagnosticEntry> {
    diagnostics.iter().map(to_manifest_entry).collect()
}

/// Converts analyzer evidence entries to manifest evidence entries.
///
/// Maps `analyzer::diagnostic::Evidence` to `manifest::types::EvidenceEntry`
/// preserving all structured fields (observation, location, context).
fn convert_evidence(evidence: &[Evidence]) -> Vec<EvidenceEntry> {
    evidence
        .iter()
        .map(|e| EvidenceEntry {
            observation: e.observation.clone(),
            location: e.location.clone(),
            context: e.context.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analyzer::diagnostic::*;

    fn make_diagnostic(
        pattern: DiagnosticPattern,
        severity: Severity,
        confidence: Confidence,
    ) -> Diagnostic {
        Diagnostic {
            id: "D001".to_string(),
            pattern,
            severity,
            confidence,
            entity: "mod::fn".to_string(),
            message: "Test diagnostic message".to_string(),
            evidence: vec![Evidence {
                observation: "0 callers in 5 files".to_string(),
                location: Some("a.py:10".to_string()),
                context: Some("private".to_string()),
            }],
            suggestion: "Remove the function or add a caller.".to_string(),
            location: "file.py:10".to_string(),
        }
    }

    // -- 1.1 Full field conversion --

    #[test]
    fn test_conversion_preserves_all_fields() {
        let diag = make_diagnostic(
            DiagnosticPattern::DataDeadEnd,
            Severity::Warning,
            Confidence::High,
        );
        let entry = to_manifest_entry(&diag);

        assert_eq!(entry.id, "D001");
        assert_eq!(entry.pattern, "data_dead_end");
        assert_eq!(entry.severity, "warning");
        assert_eq!(entry.confidence, "high");
        assert_eq!(entry.entity, "mod::fn");
        assert_eq!(entry.message, "Test diagnostic message");
        assert_eq!(entry.suggestion, "Remove the function or add a caller.");
        assert_eq!(entry.loc, "file.py:10");
        // Evidence is structured Vec<EvidenceEntry>
        assert_eq!(entry.evidence.len(), 1);
        assert_eq!(entry.evidence[0].observation, "0 callers in 5 files");
        assert_eq!(entry.evidence[0].location, Some("a.py:10".to_string()));
        assert_eq!(entry.evidence[0].context, Some("private".to_string()));
    }

    // -- 1.2 Multi-evidence conversion --

    #[test]
    fn test_conversion_preserves_multi_evidence() {
        let diag = Diagnostic {
            id: "D002".to_string(),
            pattern: DiagnosticPattern::IsolatedCluster,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity: "cluster".to_string(),
            message: "Isolated cluster found".to_string(),
            evidence: vec![
                Evidence {
                    observation: "0 callers in 5 files".to_string(),
                    location: Some("a.py:10".to_string()),
                    context: Some("private".to_string()),
                },
                Evidence {
                    observation: "defined but unused".to_string(),
                    location: None,
                    context: None,
                },
                Evidence {
                    observation: "cluster of 3".to_string(),
                    location: Some("b.py:1".to_string()),
                    context: Some("isolated".to_string()),
                },
            ],
            suggestion: "Wire this module.".to_string(),
            location: "module.py:1".to_string(),
        };
        let entry = to_manifest_entry(&diag);

        // All three evidence entries should be present
        assert_eq!(entry.evidence.len(), 3);
        assert_eq!(entry.evidence[0].observation, "0 callers in 5 files");
        assert_eq!(entry.evidence[0].location, Some("a.py:10".to_string()));
        assert_eq!(entry.evidence[1].observation, "defined but unused");
        assert_eq!(entry.evidence[1].location, None);
        assert_eq!(entry.evidence[2].observation, "cluster of 3");
        assert_eq!(entry.evidence[2].location, Some("b.py:1".to_string()));
    }

    // -- 1.3 Severity display --

    #[test]
    fn test_severity_display_all_variants_lowercase() {
        assert_eq!(format!("{}", Severity::Info), "info");
        assert_eq!(format!("{}", Severity::Warning), "warning");
        assert_eq!(format!("{}", Severity::Critical), "critical");
    }

    #[test]
    fn test_severity_display_roundtrip_with_from_str() {
        for sev in [Severity::Info, Severity::Warning, Severity::Critical] {
            let s = format!("{}", sev);
            assert_eq!(
                Severity::from_str_checked(&s),
                Some(sev),
                "Display output must round-trip through from_str_checked"
            );
        }
    }

    // -- 1.4 Confidence display --

    #[test]
    fn test_confidence_display_all_variants_lowercase() {
        assert_eq!(format!("{}", Confidence::Low), "low");
        assert_eq!(format!("{}", Confidence::Moderate), "moderate");
        assert_eq!(format!("{}", Confidence::High), "high");
    }

    #[test]
    fn test_confidence_display_roundtrip_with_from_str() {
        for conf in [Confidence::Low, Confidence::Moderate, Confidence::High] {
            let s = format!("{}", conf);
            assert_eq!(
                Confidence::from_str_checked(&s),
                Some(conf),
                "Display output must round-trip through from_str_checked"
            );
        }
    }

    // -- 1.5 All 13 pattern names convert --

    #[test]
    fn test_conversion_handles_all_13_pattern_variants() {
        let patterns = vec![
            DiagnosticPattern::IsolatedCluster,
            DiagnosticPattern::DataDeadEnd,
            DiagnosticPattern::PartialWiring,
            DiagnosticPattern::OrphanedImplementation,
            DiagnosticPattern::Duplication,
            DiagnosticPattern::ContractMismatch,
            DiagnosticPattern::CircularDependency,
            DiagnosticPattern::LayerViolation,
            DiagnosticPattern::IncompleteMigration,
            DiagnosticPattern::AsymmetricHandling,
            DiagnosticPattern::StaleReference,
            DiagnosticPattern::PhantomDependency,
            DiagnosticPattern::MissingReexport,
        ];
        for pattern in patterns {
            let diag = Diagnostic {
                id: "D999".into(),
                pattern,
                severity: Severity::Info,
                confidence: Confidence::Low,
                entity: "test".into(),
                message: "test".into(),
                evidence: vec![Evidence {
                    observation: "test".into(),
                    location: None,
                    context: None,
                }],
                suggestion: "test".into(),
                location: "test.py:1".into(),
            };
            let entry = to_manifest_entry(&diag);
            assert_eq!(
                entry.pattern,
                pattern.name(),
                "Pattern {:?} should convert to '{}'",
                pattern,
                pattern.name()
            );
        }
    }

    // -- 1.6 Empty evidence --

    #[test]
    fn test_conversion_handles_empty_evidence() {
        let diag = Diagnostic {
            id: "D001".into(),
            pattern: DiagnosticPattern::DataDeadEnd,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity: "test".into(),
            message: "test".into(),
            evidence: vec![],
            suggestion: "test".into(),
            location: "test.py:1".into(),
        };
        let entry = to_manifest_entry(&diag);
        assert!(
            entry.evidence.is_empty(),
            "Empty evidence Vec should produce empty Vec"
        );
    }

    // -- 1.7 Round-trip YAML --

    #[test]
    fn test_conversion_to_yaml_roundtrip() {
        let diag = make_diagnostic(
            DiagnosticPattern::PhantomDependency,
            Severity::Info,
            Confidence::High,
        );
        let entry = to_manifest_entry(&diag);

        let yaml = serde_yaml::to_string(&entry).expect("serialize to YAML");
        let roundtripped: DiagnosticEntry =
            serde_yaml::from_str(&yaml).expect("deserialize from YAML");

        assert_eq!(entry.id, roundtripped.id);
        assert_eq!(entry.pattern, roundtripped.pattern);
        assert_eq!(entry.severity, roundtripped.severity);
        assert_eq!(entry.confidence, roundtripped.confidence);
        assert_eq!(entry.entity, roundtripped.entity);
        assert_eq!(entry.message, roundtripped.message);
        assert_eq!(entry.evidence, roundtripped.evidence);
        assert_eq!(entry.suggestion, roundtripped.suggestion);
        assert_eq!(entry.loc, roundtripped.loc);
    }

    // -- Batch conversion --

    #[test]
    fn test_to_manifest_entries_batch_conversion() {
        let diags = vec![
            make_diagnostic(
                DiagnosticPattern::DataDeadEnd,
                Severity::Warning,
                Confidence::High,
            ),
            make_diagnostic(
                DiagnosticPattern::IsolatedCluster,
                Severity::Warning,
                Confidence::Moderate,
            ),
        ];
        let entries = to_manifest_entries(&diags);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].pattern, "data_dead_end");
        assert_eq!(entries[1].pattern, "isolated_cluster");
    }
}
