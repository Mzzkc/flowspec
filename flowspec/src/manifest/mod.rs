//! Manifest output formatting — OutputFormatter trait and implementations.
//!
//! The manifest module owns the output contract: what format the analysis
//! results take when written to stdout or a file. OutputFormatter is one of
//! two sanctioned traits (per conventions.yaml) — one implementation per
//! output format (YAML, JSON, SARIF, summary).

pub mod json;
pub mod sarif;
pub mod summary;
pub mod types;
pub mod yaml;

pub use json::JsonFormatter;
pub use sarif::SarifFormatter;
pub use summary::SummaryFormatter;
pub use types::*;
pub use yaml::YamlFormatter;

use crate::error::ManifestError;

/// Minimum source bytes before the 10x size limit is enforced.
///
/// Small projects have disproportionate metadata overhead, so the
/// manifest-to-source ratio naturally exceeds 10x for tiny inputs.
/// Below this threshold, the size check is skipped.
const SIZE_CHECK_MIN_SOURCE_BYTES: u64 = 1024;

/// Maximum allowed ratio of manifest bytes to source bytes.
const SIZE_CHECK_MAX_RATIO: f64 = 10.0;

/// Validates that a serialized manifest does not exceed the 10x source size limit.
///
/// The spec constraint (`constraints.yaml:36,48`) mandates that manifests must not
/// exceed 10x the source code size. This prevents bloated output that overwhelms
/// AI agent consumers.
///
/// Returns `Ok(())` if the size is acceptable, or `Err(ManifestError::SizeLimit)` if
/// the manifest exceeds the limit. Skips the check when `source_bytes` is below
/// [`SIZE_CHECK_MIN_SOURCE_BYTES`] (small projects have high metadata overhead)
/// or when `source_bytes` is zero (empty projects).
pub fn validate_manifest_size(serialized: &str, source_bytes: u64) -> Result<(), ManifestError> {
    if source_bytes < SIZE_CHECK_MIN_SOURCE_BYTES {
        return Ok(());
    }

    let manifest_bytes = serialized.len() as u64;
    let ratio = manifest_bytes as f64 / source_bytes as f64;

    if ratio > SIZE_CHECK_MAX_RATIO {
        return Err(ManifestError::SizeLimit {
            manifest_bytes,
            source_bytes,
            ratio,
        });
    }

    Ok(())
}

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
