// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Diagnostic types — the output of every analyzer function.
//!
//! Every diagnostic carries a pattern, severity, confidence, evidence,
//! and an actionable suggestion. Evidence must be concrete (specific
//! file:line, counts, observations) — never vague.

use serde::{Deserialize, Serialize};

/// A diagnostic finding produced by a pattern detector.
///
/// Every field is mandatory. Diagnostics with missing evidence or vague
/// suggestions are considered broken. The `id` field is assigned by the
/// pattern registry when collecting results, not by individual detectors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    /// Unique ID assigned by the registry (e.g., "D001", "D002").
    pub id: String,
    /// Which of the 13 diagnostic patterns this finding belongs to.
    pub pattern: DiagnosticPattern,
    /// How severe the issue is.
    pub severity: Severity,
    /// How confident Flowspec is in this finding.
    pub confidence: Confidence,
    /// Primary entity or entities involved (symbol names).
    pub entity: String,
    /// Human/agent-readable description using failure-pattern language.
    pub message: String,
    /// Concrete proof of what Flowspec observed.
    pub evidence: Vec<Evidence>,
    /// Actionable fix suggestion.
    pub suggestion: String,
    /// Primary source location (file:line format).
    pub location: String,
}

/// A single piece of evidence supporting a diagnostic.
///
/// Evidence is always concrete — "0 callers in 15 analyzed files",
/// not "might be unused". Location and context are optional because
/// some evidence is about groups of symbols, not a single location.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    /// What Flowspec observed: counts, structural facts, specific findings.
    pub observation: String,
    /// Source location if applicable (file:line format).
    pub location: Option<String>,
    /// Additional context explaining the observation.
    pub context: Option<String>,
}

/// The 13 diagnostic patterns organized by failure pattern.
///
/// All 13 variants are defined for a stable contract even when only
/// some are implemented. The pattern registry uses this enum for
/// `--checks` filtering in the CLI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum DiagnosticPattern {
    /// Connected components with zero inbound external edges.
    IsolatedCluster,
    /// Symbols defined but never consumed.
    DataDeadEnd,
    /// Error handlers on some paths but not parallel ones.
    PartialWiring,
    /// Trait impls with no dispatch points, public methods with zero callers.
    OrphanedImplementation,
    /// Structural similarity in IR (not textual).
    Duplication,
    /// Serde annotations vs. API schemas, signatures vs. call sites.
    ContractMismatch,
    /// Cycles in the module dependency graph.
    CircularDependency,
    /// Cross-module references violating user-defined rules.
    LayerViolation,
    /// Old/new patterns coexisting with split callers.
    IncompleteMigration,
    /// Parallel functions with inconsistent treatment.
    AsymmetricHandling,
    /// Imports resolving to re-exports/shims/moved definitions.
    StaleReference,
    /// Imports where zero imported symbols are referenced downstream.
    PhantomDependency,
    /// Public symbols not re-exported through parent module.
    MissingReexport,
}

/// Severity level of a diagnostic finding.
///
/// Ordering: Info < Warning < Critical. This ordering is used by the
/// `--severity` CLI filter: `--severity warning` means "warning and above".
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Severity {
    /// Informational — suboptimal but not broken.
    Info,
    /// Warning — structural defect that will cause problems.
    Warning,
    /// Critical — breaks correctness or causes data loss.
    Critical,
}

/// Confidence level of a diagnostic finding.
///
/// Ordering: Low < Moderate < High. This ordering is used by the
/// `--confidence` CLI filter. High confidence means structural proof
/// exists. Low confidence means the finding might be a false positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Confidence {
    /// Low — may be a false positive (e.g., public function with zero internal callers).
    Low,
    /// Moderate — likely a real issue, warrants investigation.
    Moderate,
    /// High — structural proof exists (e.g., private function with zero callers).
    High,
}

impl Severity {
    /// Parse a severity string (for CLI filter compatibility).
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
    /// Parse a confidence string (for CLI filter compatibility).
    pub fn from_str_checked(s: &str) -> Option<Self> {
        match s {
            "high" => Some(Confidence::High),
            "moderate" => Some(Confidence::Moderate),
            "low" => Some(Confidence::Low),
            _ => None,
        }
    }
}

impl DiagnosticPattern {
    /// Returns the snake_case name of this pattern (for --checks filtering).
    pub fn name(&self) -> &'static str {
        match self {
            Self::IsolatedCluster => "isolated_cluster",
            Self::DataDeadEnd => "data_dead_end",
            Self::PartialWiring => "partial_wiring",
            Self::OrphanedImplementation => "orphaned_impl",
            Self::Duplication => "duplication",
            Self::ContractMismatch => "contract_mismatch",
            Self::CircularDependency => "circular_dependency",
            Self::LayerViolation => "layer_violation",
            Self::IncompleteMigration => "incomplete_migration",
            Self::AsymmetricHandling => "asymmetric_handling",
            Self::StaleReference => "stale_reference",
            Self::PhantomDependency => "phantom_dependency",
            Self::MissingReexport => "missing_reexport",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Diagnostic construction -----------------------------------------------

    #[test]
    fn test_diagnostic_construction_all_fields_populated() {
        let d = Diagnostic {
            id: "D001".to_string(),
            pattern: DiagnosticPattern::DataDeadEnd,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity: "module::unused_fn".to_string(),
            message: "Function unused_fn is never called".to_string(),
            evidence: vec![Evidence {
                observation: "0 callers in 5 analyzed files".to_string(),
                location: Some("src/utils.py:42".to_string()),
                context: Some("function is private".to_string()),
            }],
            suggestion: "Remove the function or add a caller.".to_string(),
            location: "src/utils.py:42".to_string(),
        };
        assert_eq!(d.id, "D001");
        assert_eq!(d.pattern, DiagnosticPattern::DataDeadEnd);
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.confidence, Confidence::High);
        assert!(!d.entity.is_empty());
        assert!(!d.message.is_empty());
        assert!(!d.evidence.is_empty());
        assert!(!d.suggestion.is_empty());
        assert!(!d.location.is_empty());
    }

    #[test]
    fn test_diagnostic_evidence_is_vec_not_empty() {
        let d = Diagnostic {
            id: String::new(),
            pattern: DiagnosticPattern::IsolatedCluster,
            severity: Severity::Warning,
            confidence: Confidence::High,
            entity: "cluster".to_string(),
            message: "Isolated cluster found".to_string(),
            evidence: vec![
                Evidence {
                    observation: "0 external callers".to_string(),
                    location: None,
                    context: None,
                },
                Evidence {
                    observation: "3 internal references".to_string(),
                    location: None,
                    context: None,
                },
                Evidence {
                    observation: "cluster contains: A, B, C".to_string(),
                    location: None,
                    context: None,
                },
            ],
            suggestion: "Wire this module into the rest of the codebase.".to_string(),
            location: "module.py:1".to_string(),
        };
        assert_eq!(d.evidence.len(), 3);
        for ev in &d.evidence {
            assert!(!ev.observation.is_empty());
        }
    }

    #[test]
    fn test_diagnostic_serialization_roundtrip() {
        let d = Diagnostic {
            id: "D042".to_string(),
            pattern: DiagnosticPattern::PhantomDependency,
            severity: Severity::Info,
            confidence: Confidence::High,
            entity: "os".to_string(),
            message: "Import 'os' is never used".to_string(),
            evidence: vec![Evidence {
                observation: "0 references in file".to_string(),
                location: Some("main.py:1".to_string()),
                context: None,
            }],
            suggestion: "Remove the unused import.".to_string(),
            location: "main.py:1".to_string(),
        };
        let yaml = serde_yaml::to_string(&d).expect("serialize");
        let roundtripped: Diagnostic = serde_yaml::from_str(&yaml).expect("deserialize");
        assert_eq!(d, roundtripped);
    }

    // -- Enum ordering ---------------------------------------------------------

    #[test]
    fn test_severity_ordering_critical_gt_warning_gt_info() {
        assert!(Severity::Critical > Severity::Warning);
        assert!(Severity::Warning > Severity::Info);
        assert!(Severity::Critical > Severity::Info);
    }

    #[test]
    fn test_confidence_ordering_high_gt_moderate_gt_low() {
        assert!(Confidence::High > Confidence::Moderate);
        assert!(Confidence::Moderate > Confidence::Low);
        assert!(Confidence::High > Confidence::Low);
    }

    // -- DiagnosticPattern completeness ----------------------------------------

    #[test]
    fn test_diagnostic_pattern_has_all_13_variants() {
        let variants = vec![
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
        assert_eq!(variants.len(), 13);
    }

    // -- Evidence construction -------------------------------------------------

    #[test]
    fn test_evidence_with_location_and_context() {
        let ev = Evidence {
            observation: "0 callers in 15 analyzed files".to_string(),
            location: Some("src/utils.py:42".to_string()),
            context: Some("function is pub".to_string()),
        };
        assert!(!ev.observation.is_empty());
        assert!(ev.location.is_some());
        assert!(ev.context.is_some());
    }

    #[test]
    fn test_evidence_with_observation_only() {
        let ev = Evidence {
            observation: "module has 3 internal references".to_string(),
            location: None,
            context: None,
        };
        assert!(!ev.observation.is_empty());
        assert!(ev.location.is_none());
        assert!(ev.context.is_none());
    }

    // -- from_str_checked methods ----------------------------------------------

    #[test]
    fn test_severity_from_str_checked() {
        assert_eq!(
            Severity::from_str_checked("critical"),
            Some(Severity::Critical)
        );
        assert_eq!(
            Severity::from_str_checked("warning"),
            Some(Severity::Warning)
        );
        assert_eq!(Severity::from_str_checked("info"), Some(Severity::Info));
        assert_eq!(Severity::from_str_checked("unknown"), None);
    }

    #[test]
    fn test_confidence_from_str_checked() {
        assert_eq!(Confidence::from_str_checked("high"), Some(Confidence::High));
        assert_eq!(
            Confidence::from_str_checked("moderate"),
            Some(Confidence::Moderate)
        );
        assert_eq!(Confidence::from_str_checked("low"), Some(Confidence::Low));
        assert_eq!(Confidence::from_str_checked("unknown"), None);
    }
}
