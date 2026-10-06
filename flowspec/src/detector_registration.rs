//! Actual production detector callback construction above pure catalog metadata.
use crate::analyzer::diagnostic::Diagnostic;
use crate::analyzer::patterns::{
    asymmetric_handling, circular_dependency, contract_mismatch, data_dead_end, duplication,
    incomplete_migration, isolated_cluster, layer_violation, missing_reexport,
    orphaned_implementation, partial_wiring, phantom_dependency, stale_reference,
};
use crate::catalog::DiagnosticPattern;
use crate::graph::Graph;
use serde_json::{json, Value};
use std::path::Path;
/// Production detector callback.
pub type Detector = fn(&Graph, &Path) -> Vec<Diagnostic>;

/// Stable catalog order and preserved historical dispatch order are distinct.
#[derive(Clone)]
pub struct PatternDescriptor {
    pub pattern: DiagnosticPattern,
    pub id: &'static str,
    pub dispatch_order: usize,
    pub implementation_id: &'static str,
    pub required_capabilities: &'static [&'static str],
    pub applicability: &'static str,
    pub detector: Option<Detector>,
}
impl PatternDescriptor {
    pub fn metadata(&self) -> Value {
        json!({"id":self.id,"dispatch_order":self.dispatch_order,"implementation_id":self.implementation_id,
        "required_capabilities":self.required_capabilities,"applicability":self.applicability,"default_enabled":true,
        "detector_version":env!("CARGO_PKG_VERSION")})
    }
}

macro_rules! diagnostic_catalog {
    ($(($variant:ident, $id:literal, $module:ident, $order:literal, [$($cap:literal),*], $app:literal)),*) => {
        /// Canonical descriptors bind each name to its actual production detector.
        pub fn pattern_descriptors() -> Vec<PatternDescriptor> { vec![$(PatternDescriptor {
            pattern:DiagnosticPattern::$variant, id:$id, dispatch_order:$order,
            implementation_id:concat!("flowspec.detector.", $id), required_capabilities:&[$($cap),*],
            applicability:$app, detector:Some($module::detect)
        }),*] }
    }
}
crate::catalog::with_diagnostic_catalog!(diagnostic_catalog);
