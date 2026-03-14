//! Manifest output formatting — OutputFormatter trait and implementations.
//!
//! The manifest module owns the output contract: what format the analysis
//! results take when written to stdout or a file. OutputFormatter is one of
//! two sanctioned traits (per conventions.yaml) — one implementation per
//! output format (YAML, JSON, SARIF, summary).

pub mod json;
pub mod types;
pub mod yaml;

pub use json::JsonFormatter;
pub use types::*;
pub use yaml::YamlFormatter;

use crate::error::ManifestError;

/// Trait for formatting analysis output. One implementation per output format.
///
/// Implementations must produce valid, parseable output in their format.
/// The manifest is the full analysis output; diagnostics is the filtered
/// output used by the `diagnose` command.
pub trait OutputFormatter {
    /// Format a complete manifest (for the `analyze` command).
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError>;

    /// Format only diagnostics (for the `diagnose` command).
    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError>;
}
