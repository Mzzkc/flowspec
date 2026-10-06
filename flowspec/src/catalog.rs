//! Canonical diagnostic identities and metadata, independent of detector wiring.
use serde::{Deserialize, Serialize};
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

/// Source-independent registration metadata for one canonical diagnostic.
#[derive(Clone, Copy)]
pub struct PatternMetadata {
    pub pattern: DiagnosticPattern,
    pub id: &'static str,
    pub dispatch_order: usize,
    pub implementation_id: &'static str,
    pub required_capabilities: &'static [&'static str],
    pub applicability: &'static str,
}
macro_rules! metadata_catalog {
    ($(($variant:ident, $id:literal, $module:ident, $order:literal, [$($cap:literal),*], $app:literal)),*) => {
        pub const PATTERN_IDS: [&str; 13] = [$($id),*];
        pub const PATTERN_METADATA: [PatternMetadata; 13] = [$(PatternMetadata {
            pattern: DiagnosticPattern::$variant, id: $id, dispatch_order: $order,
            implementation_id: concat!("flowspec.detector.", $id), required_capabilities: &[$($cap),*], applicability: $app,
        }),*];
        impl DiagnosticPattern {
            pub fn name(&self) -> &'static str { match self { $(Self::$variant => $id),* } }
        }
    }
}
macro_rules! with_diagnostic_catalog {
    ($consumer:ident) => {
        $consumer!(
            (
                IsolatedCluster,
                "isolated_cluster",
                isolated_cluster,
                0,
                ["symbols", "calls", "resolution"],
                "selected_language"
            ),
            (
                DataDeadEnd,
                "data_dead_end",
                data_dead_end,
                1,
                [
                    "symbols",
                    "calls",
                    "value_references",
                    "exports",
                    "registration",
                    "consumer_closure"
                ],
                "selected_language"
            ),
            (
                PartialWiring,
                "partial_wiring",
                partial_wiring,
                10,
                ["symbols", "calls", "control_flow"],
                "selected_language"
            ),
            (
                OrphanedImplementation,
                "orphaned_impl",
                orphaned_implementation,
                4,
                ["symbols", "calls", "resolution", "types", "implementations"],
                "selected_language"
            ),
            (
                Duplication,
                "duplication",
                duplication,
                11,
                ["symbols", "binding_structure"],
                "selected_language"
            ),
            (
                ContractMismatch,
                "contract_mismatch",
                contract_mismatch,
                8,
                ["symbols", "calls", "control_flow", "types"],
                "selected_language"
            ),
            (
                CircularDependency,
                "circular_dependency",
                circular_dependency,
                3,
                ["imports", "dependency_edges"],
                "selected_language"
            ),
            (
                LayerViolation,
                "layer_violation",
                layer_violation,
                6,
                ["imports", "dependency_edges"],
                "explicit_layer_policy"
            ),
            (
                IncompleteMigration,
                "incomplete_migration",
                incomplete_migration,
                9,
                ["imports", "resolution"],
                "selected_language"
            ),
            (
                AsymmetricHandling,
                "asymmetric_handling",
                asymmetric_handling,
                12,
                ["symbols", "calls", "control_flow"],
                "selected_language"
            ),
            (
                StaleReference,
                "stale_reference",
                stale_reference,
                7,
                ["imports", "resolution"],
                "selected_language"
            ),
            (
                PhantomDependency,
                "phantom_dependency",
                phantom_dependency,
                2,
                ["imports", "exports", "value_references"],
                "selected_language"
            ),
            (
                MissingReexport,
                "missing_reexport",
                missing_reexport,
                5,
                ["imports", "exports", "value_references"],
                "selected_language"
            )
        );
    };
}
pub(crate) use with_diagnostic_catalog;
with_diagnostic_catalog!(metadata_catalog);
/// Names in the preserved public/error catalog order.
pub fn pattern_names() -> String {
    PATTERN_IDS.join(", ")
}
