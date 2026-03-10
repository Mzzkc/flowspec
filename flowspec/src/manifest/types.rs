//! Manifest data model — all 8 required sections per manifest-schema.yaml.
//!
//! Field names use abbreviated forms (vis, sig, loc) for token efficiency.
//! Structs are ordered so serde_yaml serializes sections in priority order:
//! metadata → summary → diagnostics → entities → flows → boundaries →
//! dependency_graph → type_flows (most valuable first).

use serde::{Deserialize, Serialize};

/// Complete analysis manifest — the primary output of `flowspec analyze`.
///
/// Contains all 8 required sections. Sections are always present even when
/// empty (empty lists, not omitted) so consumers can rely on a stable schema.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Project identity, analysis timestamp, version, counts.
    pub metadata: Metadata,
    /// Compressed structural overview (~2K token budget).
    pub summary: Summary,
    /// Structural issues found in the codebase.
    pub diagnostics: Vec<DiagnosticEntry>,
    /// Every meaningful symbol in the codebase.
    pub entities: Vec<EntityEntry>,
    /// Traced data flow paths from entry to exit.
    pub flows: Vec<FlowEntry>,
    /// Interfaces where data crosses a meaningful boundary.
    pub boundaries: Vec<BoundaryEntry>,
    /// Module-level dependency structure.
    pub dependency_graph: Vec<DependencyEdge>,
    /// Where each significant type is created, transformed, and consumed.
    pub type_flows: Vec<TypeFlowEntry>,
}

/// Project identity, analysis timestamp, version, and aggregate counts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Metadata {
    /// Project name (derived from directory name or config).
    pub project: String,
    /// ISO 8601 timestamp of when analysis was performed.
    pub analyzed_at: String,
    /// Flowspec version that produced this manifest.
    pub flowspec_version: String,
    /// Languages detected and analyzed.
    pub languages: Vec<String>,
    /// Total number of files analyzed.
    pub file_count: u64,
    /// Total number of entities found.
    pub entity_count: u64,
    /// Total number of flows traced.
    pub flow_count: u64,
    /// Total number of diagnostics reported.
    pub diagnostic_count: u64,
    /// Whether this was an incremental analysis.
    pub incremental: bool,
    /// Number of files re-analyzed (0 if full run).
    pub files_changed: u64,
}

/// Compressed structural overview. An agent reading only this section
/// should understand the project's architecture, key flows, and issues.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    /// 2-4 sentence description of the project's structure.
    pub architecture: String,
    /// Each module with count and one-line role description.
    pub modules: Vec<ModuleSummary>,
    /// Where data enters the system.
    pub entry_points: Vec<String>,
    /// Where data leaves the system.
    pub exit_points: Vec<String>,
    /// The most significant data flow paths, compressed.
    pub key_flows: Vec<KeyFlow>,
    /// Diagnostic counts by severity plus top issues.
    pub diagnostic_summary: DiagnosticSummary,
}

/// A module's summary entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleSummary {
    /// Module name.
    pub name: String,
    /// Number of entities in this module.
    pub entity_count: u64,
    /// One-line role description.
    pub role: String,
}

/// A key data flow path summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyFlow {
    /// Flow name.
    pub name: String,
    /// Compressed path summary.
    pub path_summary: String,
}

/// Diagnostic counts by severity plus top issues as one-liners.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticSummary {
    /// Number of critical diagnostics.
    pub critical: u64,
    /// Number of warning diagnostics.
    pub warning: u64,
    /// Number of info diagnostics.
    pub info: u64,
    /// Top issues as one-line summaries (max 5).
    pub top_issues: Vec<String>,
}

/// A single entity (symbol) entry using abbreviated field names.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityEntry {
    /// Unique identifier: module_path::name.
    pub id: String,
    /// Symbol kind: fn, method, struct, class, trait, interface, module, var, const, macro, enum.
    pub kind: String,
    /// Visibility: pub, priv, crate, protected.
    pub vis: String,
    /// Compact signature: (param_types) -> return_type.
    pub sig: String,
    /// Location: file:line.
    pub loc: String,
    /// IDs of symbols this entity calls.
    pub calls: Vec<String>,
    /// IDs of symbols that call this entity.
    pub called_by: Vec<String>,
    /// Decorators, derives, attributes.
    pub annotations: Vec<String>,
}

/// A single diagnostic entry with all required fields including confidence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosticEntry {
    /// Unique diagnostic ID (D001, D002, ...).
    pub id: String,
    /// Failure pattern category.
    pub pattern: String,
    /// Severity: critical, warning, info.
    pub severity: String,
    /// Confidence level: high, moderate, low.
    pub confidence: String,
    /// Primary entity or entities involved.
    pub entity: String,
    /// Human/agent-readable description of the issue.
    pub message: String,
    /// Specific proof — what Flowspec observed.
    pub evidence: String,
    /// Actionable fix suggestion.
    pub suggestion: String,
    /// Primary file:line location.
    pub loc: String,
}

/// A traced data flow path from entry point to exit point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEntry {
    /// Flow identifier.
    pub id: String,
    /// One-line summary of what this flow does.
    pub description: String,
    /// Entry point entity ID.
    pub entry: String,
    /// Exit point entity ID or description.
    pub exit: String,
    /// Ordered steps in the flow.
    pub steps: Vec<FlowStep>,
    /// Diagnostic IDs that affect this flow.
    pub issues: Vec<String>,
}

/// A single step in a data flow.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowStep {
    /// Entity involved in this step.
    pub entity: String,
    /// Action performed.
    pub action: String,
    /// Input type.
    pub in_type: String,
    /// Output type.
    pub out_type: String,
}

/// A boundary where data crosses a meaningful interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoundaryEntry {
    /// Boundary identifier.
    pub id: String,
    /// Boundary type: module, package, network, serialization, ffi.
    #[serde(rename = "type")]
    pub boundary_type: String,
    /// Source module/scope.
    pub from: String,
    /// Target module/scope.
    pub to: String,
    /// Functions that cross this boundary.
    pub crossing_points: Vec<CrossingPoint>,
    /// Diagnostic IDs at this boundary.
    pub issues: Vec<String>,
}

/// A function that crosses a boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrossingPoint {
    /// Function name.
    #[serde(rename = "fn")]
    pub func: String,
    /// Data type flowing in.
    pub data_in: String,
    /// Data type flowing out.
    pub data_out: String,
}

/// A module-level dependency edge.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyEdge {
    /// Source module.
    pub from: String,
    /// Target module.
    pub to: String,
    /// Number of cross-references.
    pub weight: u64,
    /// Direction: unidirectional or bidirectional.
    pub direction: String,
    /// Diagnostic IDs (e.g., circular deps).
    pub issues: Vec<String>,
}

/// Where a significant type is created, transformed, and consumed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeFlowEntry {
    /// Type name.
    #[serde(rename = "type")]
    pub type_name: String,
    /// Where instances are created.
    pub created_at: Vec<String>,
    /// How this type is transformed.
    pub transformed_to: Vec<TypeTransformation>,
    /// Where this type is consumed.
    pub consumed_by: Vec<String>,
    /// Scope: request, session, static, etc.
    pub lifetime: String,
}

/// A type transformation step.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeTransformation {
    /// Target type after transformation.
    pub target_type: String,
    /// Function/method that performs the transformation.
    pub via: String,
}

impl Manifest {
    /// Create an empty manifest with all sections present but unpopulated.
    pub fn empty() -> Self {
        Self {
            metadata: Metadata {
                project: String::new(),
                analyzed_at: String::new(),
                flowspec_version: String::new(),
                languages: Vec::new(),
                file_count: 0,
                entity_count: 0,
                flow_count: 0,
                diagnostic_count: 0,
                incremental: false,
                files_changed: 0,
            },
            summary: Summary {
                architecture: String::new(),
                modules: Vec::new(),
                entry_points: Vec::new(),
                exit_points: Vec::new(),
                key_flows: Vec::new(),
                diagnostic_summary: DiagnosticSummary {
                    critical: 0,
                    warning: 0,
                    info: 0,
                    top_issues: Vec::new(),
                },
            },
            diagnostics: Vec::new(),
            entities: Vec::new(),
            flows: Vec::new(),
            boundaries: Vec::new(),
            dependency_graph: Vec::new(),
            type_flows: Vec::new(),
        }
    }

    /// Create a sample fully populated manifest for testing.
    pub fn sample_full() -> Self {
        Self {
            metadata: Metadata {
                project: "test-project".to_string(),
                analyzed_at: "2026-03-10T12:00:00Z".to_string(),
                flowspec_version: env!("CARGO_PKG_VERSION").to_string(),
                languages: vec!["python".to_string()],
                file_count: 2,
                entity_count: 3,
                flow_count: 1,
                diagnostic_count: 1,
                incremental: false,
                files_changed: 0,
            },
            summary: Summary {
                architecture: "Single-module Python project with a main entry point.".to_string(),
                modules: vec![ModuleSummary {
                    name: "main".to_string(),
                    entity_count: 3,
                    role: "Entry point and core logic".to_string(),
                }],
                entry_points: vec!["main::main".to_string()],
                exit_points: vec!["main::main".to_string()],
                key_flows: vec![KeyFlow {
                    name: "main flow".to_string(),
                    path_summary: "main -> greet -> print".to_string(),
                }],
                diagnostic_summary: DiagnosticSummary {
                    critical: 0,
                    warning: 1,
                    info: 0,
                    top_issues: vec!["dead_function has 0 callers".to_string()],
                },
            },
            diagnostics: vec![DiagnosticEntry::sample_warning()],
            entities: vec![
                EntityEntry {
                    id: "main::greet".to_string(),
                    kind: "fn".to_string(),
                    vis: "pub".to_string(),
                    sig: "(name: str) -> str".to_string(),
                    loc: "main.py:3".to_string(),
                    calls: vec![],
                    called_by: vec!["main::main".to_string()],
                    annotations: vec![],
                },
                EntityEntry {
                    id: "main::main".to_string(),
                    kind: "fn".to_string(),
                    vis: "priv".to_string(),
                    sig: "() -> None".to_string(),
                    loc: "main.py:7".to_string(),
                    calls: vec!["main::greet".to_string()],
                    called_by: vec![],
                    annotations: vec![],
                },
            ],
            flows: vec![FlowEntry {
                id: "F001".to_string(),
                description: "Main execution flow".to_string(),
                entry: "main::main".to_string(),
                exit: "main::main".to_string(),
                steps: vec![FlowStep {
                    entity: "main::main".to_string(),
                    action: "call".to_string(),
                    in_type: "None".to_string(),
                    out_type: "str".to_string(),
                }],
                issues: vec![],
            }],
            boundaries: vec![],
            dependency_graph: vec![],
            type_flows: vec![],
        }
    }
}

impl DiagnosticEntry {
    /// Create a sample critical diagnostic for testing.
    pub fn sample_critical() -> Self {
        Self {
            id: "D001".to_string(),
            pattern: "data_dead_end".to_string(),
            severity: "critical".to_string(),
            confidence: "high".to_string(),
            entity: "module::dead_function".to_string(),
            message: "Function dead_function is never called".to_string(),
            evidence: "Function `dead_function` at `dead_code.py:8` has 0 callers across 2 analyzed files".to_string(),
            suggestion: "Remove the function or add a caller. If intentionally unused, add a # flowspec:ignore comment.".to_string(),
            loc: "dead_code.py:8".to_string(),
        }
    }

    /// Create a sample warning diagnostic for testing.
    pub fn sample_warning() -> Self {
        Self {
            id: "D002".to_string(),
            pattern: "data_dead_end".to_string(),
            severity: "warning".to_string(),
            confidence: "high".to_string(),
            entity: "dead_code::dead_function".to_string(),
            message: "Function dead_function is never called".to_string(),
            evidence: "Function `dead_function` at `dead_code.py:8` has 0 callers across 2 analyzed files".to_string(),
            suggestion: "Remove the function or add a caller. If intentionally unused, add a # flowspec:ignore comment.".to_string(),
            loc: "dead_code.py:8".to_string(),
        }
    }
}
