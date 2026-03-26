//! Library-level error types for Flowspec.
//!
//! Every error carries what failed, why, and a suggestion for how to fix it.
//! This is critical for both human operators and AI agents that parse errors.

use std::path::PathBuf;

/// Top-level error type for the Flowspec library crate.
///
/// Each variant carries enough context to diagnose the failure without
/// reading source code. Error messages include actionable fix suggestions.
#[derive(Debug, thiserror::Error)]
pub enum FlowspecError {
    /// A source file could not be parsed.
    #[error("parse error in {file}: {reason}")]
    Parse {
        /// Path to the file that failed to parse.
        file: PathBuf,
        /// Human-readable explanation of what went wrong.
        reason: String,
    },

    /// Configuration is invalid or missing.
    #[error("configuration error: {reason} (fix: {suggestion})")]
    Config {
        /// What is wrong with the configuration.
        reason: String,
        /// How to fix the configuration issue.
        suggestion: String,
    },

    /// Manifest formatting or size constraint failure.
    #[error("manifest error: {reason}")]
    Manifest {
        /// What went wrong during manifest generation.
        reason: String,
    },

    /// An I/O operation failed.
    #[error("I/O error accessing {path}: {source}")]
    Io {
        /// Path that was being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },

    /// The analysis target path does not exist or is inaccessible.
    #[error(
        "target path does not exist: {path} (fix: check that the path exists and is readable)"
    )]
    TargetNotFound {
        /// Path that was not found.
        path: PathBuf,
    },

    /// A requested output format is not yet implemented.
    #[error("{format} format is not yet implemented (fix: use --format yaml or --format json)")]
    FormatNotImplemented {
        /// The format that was requested.
        format: String,
    },

    /// A requested command is not yet implemented.
    #[error("{command} command is not yet implemented (fix: {suggestion})")]
    CommandNotImplemented {
        /// The command that was requested.
        command: String,
        /// Actionable suggestion for what to do instead.
        suggestion: String,
    },

    /// An unsupported language was requested.
    #[error(
        "unsupported language: {language} (fix: supported languages are python, javascript, rust)"
    )]
    UnsupportedLanguage {
        /// The language that was requested.
        language: String,
    },

    /// An unknown diagnostic pattern was specified in --checks.
    #[error("unknown diagnostic pattern: {pattern} (fix: valid patterns are isolated_cluster, data_dead_end, phantom_dependency, orphaned_impl, circular_dependency, missing_reexport, contract_mismatch, stale_reference, layer_violation, duplication, partial_wiring, asymmetric_handling, incomplete_migration)")]
    UnknownPattern {
        /// The pattern name that was not recognized.
        pattern: String,
    },

    /// The analysis target path is empty.
    #[error("empty path provided (fix: provide a valid project path, e.g. 'flowspec analyze .')")]
    EmptyPath,

    /// An operation on the analysis graph failed.
    #[error("graph error: {0}")]
    Graph(String),

    /// A referenced symbol was not found in the graph.
    #[error("symbol not found: {0}")]
    SymbolNotFound(String),
}

/// Error type specific to manifest formatting operations.
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// Serialization failed (YAML, JSON, or other format).
    #[error("serialization failed: {reason}")]
    Serialization {
        /// What went wrong during serialization.
        reason: String,
    },

    /// Manifest exceeds the format-specific source code size limit.
    #[error("manifest size ({manifest_bytes} bytes) exceeds {limit}x source size ({source_bytes} bytes). Ratio: {ratio:.1}x")]
    SizeLimit {
        /// Manifest size in bytes.
        manifest_bytes: u64,
        /// Source code size in bytes.
        source_bytes: u64,
        /// The actual ratio.
        ratio: f64,
        /// The format-specific ratio limit that was exceeded.
        limit: f64,
    },
}

impl From<ManifestError> for FlowspecError {
    fn from(err: ManifestError) -> Self {
        FlowspecError::Manifest {
            reason: err.to_string(),
        }
    }
}
