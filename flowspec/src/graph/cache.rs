// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Graph cache persistence — serialization, file hashing, and cache metadata.
//!
//! Provides the infrastructure for incremental analysis: save/load the analysis
//! graph to disk, track file content hashes for change detection, and manage
//! cache metadata for version-based invalidation.
//!
//! ## Cache directory layout
//!
//! ```text
//! .flowspec/cache/
//! ├── graph.bin          # bincode-serialized Graph (via serde compat layer)
//! ├── file_hashes.json   # { "path": "sha256hex", ... }
//! └── metadata.json      # { "flowspec_version": "0.1.0", "timestamp": "..." }
//! ```
//!
//! All writes use atomic temp-file-and-rename to prevent partial files on crash.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::FlowspecError;

const GRAPH_FILE: &str = "graph.bin";
const GRAPH_TMP_FILE: &str = "graph.bin.tmp";
const FILE_HASHES_FILE: &str = "file_hashes.json";
const METADATA_FILE: &str = "metadata.json";

/// Cache metadata for invalidation across Flowspec versions.
///
/// If the `flowspec_version` in a cached `metadata.json` doesn't match the
/// running binary's version, the entire cache should be discarded and a full
/// re-analysis performed. This prevents silent corruption from format changes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMetadata {
    /// The Flowspec version that produced this cache (e.g., "0.1.0").
    pub flowspec_version: String,
    /// ISO 8601 timestamp of when the cache was written.
    pub timestamp: String,
}

impl CacheMetadata {
    /// Saves metadata to `cache_dir/metadata.json` using atomic write.
    ///
    /// Creates `cache_dir` if it doesn't exist.
    pub fn save(&self, cache_dir: &Path) -> Result<(), FlowspecError> {
        fs::create_dir_all(cache_dir).map_err(|e| FlowspecError::Cache {
            path: cache_dir.to_path_buf(),
            reason: format!("failed to create cache directory: {e}"),
        })?;

        let json = serde_json::to_string_pretty(self).map_err(|e| FlowspecError::Cache {
            path: cache_dir.join(METADATA_FILE),
            reason: format!("failed to serialize metadata: {e}"),
        })?;

        let tmp_path = cache_dir.join(format!("{METADATA_FILE}.tmp"));
        let final_path = cache_dir.join(METADATA_FILE);

        fs::write(&tmp_path, json.as_bytes()).map_err(|e| FlowspecError::Cache {
            path: tmp_path.clone(),
            reason: format!("failed to write temp metadata file: {e}"),
        })?;

        fs::rename(&tmp_path, &final_path).map_err(|e| FlowspecError::Cache {
            path: final_path,
            reason: format!("failed to rename metadata temp file: {e}"),
        })?;

        Ok(())
    }

    /// Loads metadata from `cache_dir/metadata.json`.
    ///
    /// Returns `Err` if the file is missing or corrupt — never panics.
    pub fn load(cache_dir: &Path) -> Result<Self, FlowspecError> {
        let path = cache_dir.join(METADATA_FILE);
        let data = fs::read_to_string(&path).map_err(|e| FlowspecError::Cache {
            path: path.clone(),
            reason: format!("failed to read metadata file: {e}"),
        })?;

        serde_json::from_str(&data).map_err(|e| FlowspecError::Cache {
            path,
            reason: format!("failed to deserialize metadata: {e}"),
        })
    }
}

/// Computes SHA256 hashes for a list of source files.
///
/// Returns a map from file path to lowercase hex-encoded SHA256 digest.
/// Returns `Err` if any file cannot be read (files can disappear between
/// directory listing and hash computation).
pub fn compute_file_hashes(paths: &[PathBuf]) -> Result<HashMap<PathBuf, String>, FlowspecError> {
    let mut hashes = HashMap::with_capacity(paths.len());

    for path in paths {
        let mut file = fs::File::open(path).map_err(|e| FlowspecError::Cache {
            path: path.clone(),
            reason: format!("failed to open file for hashing: {e}"),
        })?;

        let mut hasher = Sha256::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = file.read(&mut buf).map_err(|e| FlowspecError::Cache {
                path: path.clone(),
                reason: format!("failed to read file for hashing: {e}"),
            })?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }

        let digest = hasher.finalize();
        let hex = digest.iter().fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write;
            let _ = write!(s, "{b:02x}");
            s
        });
        hashes.insert(path.clone(), hex);
    }

    Ok(hashes)
}

/// Saves file hashes to `cache_dir/file_hashes.json` using atomic write.
pub fn save_file_hashes(
    hashes: &HashMap<PathBuf, String>,
    cache_dir: &Path,
) -> Result<(), FlowspecError> {
    fs::create_dir_all(cache_dir).map_err(|e| FlowspecError::Cache {
        path: cache_dir.to_path_buf(),
        reason: format!("failed to create cache directory: {e}"),
    })?;

    let json = serde_json::to_string_pretty(hashes).map_err(|e| FlowspecError::Cache {
        path: cache_dir.join(FILE_HASHES_FILE),
        reason: format!("failed to serialize file hashes: {e}"),
    })?;

    let tmp_path = cache_dir.join(format!("{FILE_HASHES_FILE}.tmp"));
    let final_path = cache_dir.join(FILE_HASHES_FILE);

    fs::write(&tmp_path, json.as_bytes()).map_err(|e| FlowspecError::Cache {
        path: tmp_path.clone(),
        reason: format!("failed to write temp file hashes: {e}"),
    })?;

    fs::rename(&tmp_path, &final_path).map_err(|e| FlowspecError::Cache {
        path: final_path,
        reason: format!("failed to rename file hashes temp file: {e}"),
    })?;

    Ok(())
}

/// Loads file hashes from `cache_dir/file_hashes.json`.
pub fn load_file_hashes(cache_dir: &Path) -> Result<HashMap<PathBuf, String>, FlowspecError> {
    let path = cache_dir.join(FILE_HASHES_FILE);
    let data = fs::read_to_string(&path).map_err(|e| FlowspecError::Cache {
        path: path.clone(),
        reason: format!("failed to read file hashes: {e}"),
    })?;

    serde_json::from_str(&data).map_err(|e| FlowspecError::Cache {
        path,
        reason: format!("failed to deserialize file hashes: {e}"),
    })
}

/// Serializes the graph to `cache_dir/graph.bin` using bincode via serde compat.
///
/// Creates `cache_dir` if it doesn't exist. Uses atomic write (temp file +
/// rename) so a crash mid-write never leaves a partial `graph.bin`.
pub(super) fn save_graph(graph: &super::Graph, cache_dir: &Path) -> Result<(), FlowspecError> {
    fs::create_dir_all(cache_dir).map_err(|e| FlowspecError::Cache {
        path: cache_dir.to_path_buf(),
        reason: format!("failed to create cache directory: {e}"),
    })?;

    let encoded =
        bincode::serde::encode_to_vec(graph, bincode::config::standard()).map_err(|e| {
            FlowspecError::Cache {
                path: cache_dir.join(GRAPH_FILE),
                reason: format!("failed to serialize graph: {e}"),
            }
        })?;

    let tmp_path = cache_dir.join(GRAPH_TMP_FILE);
    let final_path = cache_dir.join(GRAPH_FILE);

    fs::write(&tmp_path, &encoded).map_err(|e| FlowspecError::Cache {
        path: tmp_path.clone(),
        reason: format!("failed to write temp graph file: {e}"),
    })?;

    fs::rename(&tmp_path, &final_path).map_err(|e| FlowspecError::Cache {
        path: final_path,
        reason: format!("failed to rename graph temp file: {e}"),
    })?;

    Ok(())
}

/// Deserializes a graph from `cache_dir/graph.bin`.
///
/// Returns `Err` on missing file, corrupt data, or deserialization failure —
/// never panics. Callers should fall back to full re-analysis on error.
pub(super) fn load_graph(cache_dir: &Path) -> Result<super::Graph, FlowspecError> {
    let path = cache_dir.join(GRAPH_FILE);
    let data = fs::read(&path).map_err(|e| FlowspecError::Cache {
        path: path.clone(),
        reason: format!("failed to read graph file: {e}"),
    })?;

    if data.is_empty() {
        return Err(FlowspecError::Cache {
            path,
            reason: "graph file is empty — cache may be corrupt".to_string(),
        });
    }

    let (graph, _len): (super::Graph, usize) =
        bincode::serde::decode_from_slice(&data, bincode::config::standard()).map_err(|e| {
            FlowspecError::Cache {
                path,
                reason: format!("failed to deserialize graph (cache may be corrupt): {e}"),
            }
        })?;

    Ok(graph)
}
