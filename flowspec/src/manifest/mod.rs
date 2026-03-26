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
//! queries. Output is validated by [`validate_manifest_size`] to enforce
//! format-specific source-size constraints (with a 20KB floor for small projects).

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

/// Maximum allowed ratio of manifest bytes to source bytes (YAML/default).
///
/// Format-specific thresholds are computed by [`max_ratio_for_format`].
/// This constant is the baseline for YAML and unknown formats.
const SIZE_CHECK_MAX_RATIO: f64 = 10.0;

/// Returns the maximum allowed manifest-to-source ratio for a given output format.
///
/// Different formats have inherent overhead that affects the ratio:
/// - **YAML** (10x): Most compact structured format. Baseline limit.
/// - **JSON** (15x): ~1.2-1.5x overhead from braces, quotes, commas.
/// - **SARIF** (20x): ~1.3-1.8x overhead from rule metadata, result objects, tool info.
/// - **Summary**: Exempt — always small (~2K tokens), no meaningful ratio risk.
/// - **Unknown** (10x): Strictest limit as fail-safe for unrecognized format strings.
fn max_ratio_for_format(format: &str) -> f64 {
    match format {
        "json" => 15.0,
        "sarif" => 20.0,
        "summary" => f64::MAX,
        _ => SIZE_CHECK_MAX_RATIO, // yaml and unknown formats use strictest limit
    }
}

/// Minimum manifest size (in bytes) that is always allowed regardless of ratio.
///
/// Manifests under 20KB are small enough that they pose no risk of overwhelming
/// AI agent consumers, even if their ratio to source code exceeds 10x. This
/// protects small projects with high metadata overhead from spurious rejections.
const MIN_MANIFEST_ALLOW_BYTES: u64 = 20_480;

/// Validates that a serialized manifest does not exceed the format-specific source size limit.
///
/// The spec constraint (`constraints.yaml:36,48`) mandates that manifests must not
/// exceed a format-dependent multiple of the source code size. This prevents bloated
/// output that overwhelms AI agent consumers.
///
/// Per-format thresholds account for inherent format overhead:
/// - YAML: 10x (most compact, baseline)
/// - JSON: 15x (braces, quotes, commas add ~1.2-1.5x)
/// - SARIF: 20x (rule metadata, result objects, tool info add ~1.3-1.8x)
/// - Summary: exempt (always small)
///
/// Returns `Ok(())` if the size is acceptable, or `Err(ManifestError::SizeLimit)` if
/// the manifest exceeds the limit. Skips the check when:
/// - `source_bytes` is below [`SIZE_CHECK_MIN_SOURCE_BYTES`] (tiny projects)
/// - manifest size is below [`MIN_MANIFEST_ALLOW_BYTES`] (small manifests always allowed)
pub fn validate_manifest_size(
    serialized: &str,
    source_bytes: u64,
    format: &str,
) -> Result<(), ManifestError> {
    if source_bytes < SIZE_CHECK_MIN_SOURCE_BYTES {
        return Ok(());
    }

    let manifest_bytes = serialized.len() as u64;

    if manifest_bytes < MIN_MANIFEST_ALLOW_BYTES {
        return Ok(());
    }

    let ratio = manifest_bytes as f64 / source_bytes as f64;
    let limit = max_ratio_for_format(format);

    if ratio > limit {
        return Err(ManifestError::SizeLimit {
            manifest_bytes,
            source_bytes,
            ratio,
            limit,
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
