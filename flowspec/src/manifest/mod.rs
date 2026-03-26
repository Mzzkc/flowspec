//! Manifest output formatting — the final stage of the Flowspec pipeline.
//!
//! The manifest module owns the output contract: what format the analysis
//! results take when written to stdout or a file. [`OutputFormatter`] is one
//! of two sanctioned traits (per `conventions.yaml`) — one implementation per
//! output format (YAML, JSON, SARIF, summary). The graph is the source of
//! truth; manifests are exports of it.
//!
//! **Pipeline position:** Parser → Graph → Analyzer → Manifest.
//! Input comes from [`crate::analyzer`] diagnostics and [`crate::graph::Graph`]
//! queries. Output is validated by [`validate_manifest_size`] to enforce the
//! 10x source-size constraint (with a 20KB floor for small projects).

/// JSON output formatter.
pub mod json;
/// SARIF v2.1.0 output formatter for CI integration.
pub mod sarif;
/// Human-readable summary output formatter (~2K tokens).
pub mod summary;
/// Manifest data types — `Manifest`, `EntityEntry`, `FlowEntry`, `DiagnosticEntry`, etc.
pub mod types;
/// YAML output formatter (default format).
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

/// Minimum manifest size (in bytes) that is always allowed regardless of ratio.
///
/// Manifests under 20KB are small enough that they pose no risk of overwhelming
/// AI agent consumers, even if their ratio to source code exceeds 10x. This
/// protects small projects with high metadata overhead from spurious rejections.
const MIN_MANIFEST_ALLOW_BYTES: u64 = 20_480;

/// Validates that a serialized manifest does not exceed the 10x source size limit.
///
/// The spec constraint (`constraints.yaml:36,48`) mandates that manifests must not
/// exceed 10x the source code size. This prevents bloated output that overwhelms
/// AI agent consumers.
///
/// Returns `Ok(())` if the size is acceptable, or `Err(ManifestError::SizeLimit)` if
/// the manifest exceeds the limit. Skips the check when:
/// - `source_bytes` is below [`SIZE_CHECK_MIN_SOURCE_BYTES`] (tiny projects)
/// - manifest size is below [`MIN_MANIFEST_ALLOW_BYTES`] (small manifests always allowed)
pub fn validate_manifest_size(serialized: &str, source_bytes: u64) -> Result<(), ManifestError> {
    if source_bytes < SIZE_CHECK_MIN_SOURCE_BYTES {
        return Ok(());
    }

    let manifest_bytes = serialized.len() as u64;

    if manifest_bytes < MIN_MANIFEST_ALLOW_BYTES {
        return Ok(());
    }

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
/// This is one of two sanctioned traits in Flowspec (per `conventions.yaml`).
/// Each output format — YAML, JSON, SARIF, summary — has exactly one
/// implementation. To add a new output format, implement this trait and
/// register the formatter in `commands.rs`.
///
/// # Implementors
///
/// - [`YamlFormatter`] — default format, full manifest (agents).
/// - [`JsonFormatter`] — full manifest for tooling consumers.
/// - [`SarifFormatter`] — SARIF v2.1.0 for CI integration (GitHub Code Scanning).
/// - [`SummaryFormatter`] — compact plain-text (~2K tokens, humans).
///
/// # Contract
///
/// - `format_manifest` must produce valid, parseable output in the target format.
/// - `format_diagnostics` formats the filtered subset used by the `diagnose` command.
/// - Implementations must not alter the ordering of manifest sections.
pub trait OutputFormatter {
    /// Format a complete [`Manifest`] into a string (for the `analyze` command).
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError>;

    /// Format only diagnostics into a string (for the `diagnose` command).
    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError>;
}
