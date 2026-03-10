//! Configuration loading and validation.
//!
//! Placeholder for cycle 1 — loads config from `.flowspec/config.yaml`
//! if it exists, otherwise uses defaults. The config module will grow
//! to support language-specific settings and layer violation rules.

use std::path::{Path, PathBuf};

/// Project configuration loaded from `.flowspec/config.yaml`.
#[derive(Debug, Clone)]
pub struct Config {
    /// Path to the config file, if one was found.
    pub config_path: Option<PathBuf>,
    /// Languages to analyze (empty = auto-detect).
    pub languages: Vec<String>,
}

impl Config {
    /// Load configuration from the default location or a specified path.
    ///
    /// If `config_path` is Some, load from that path. Otherwise, look for
    /// `.flowspec/config.yaml` in the project root. If no config file exists,
    /// use defaults.
    pub fn load(
        project_root: &Path,
        config_path: Option<&Path>,
    ) -> Result<Self, crate::error::FlowspecError> {
        if let Some(path) = config_path {
            if !path.exists() {
                return Err(crate::error::FlowspecError::Config {
                    reason: format!("config file not found: {}", path.display()),
                    suggestion: format!(
                        "check that {} exists, or omit --config to use defaults",
                        path.display()
                    ),
                });
            }
            // For now, just acknowledge the config file exists
            return Ok(Self {
                config_path: Some(path.to_path_buf()),
                languages: Vec::new(),
            });
        }

        let default_path = project_root.join(".flowspec").join("config.yaml");
        if default_path.exists() {
            Ok(Self {
                config_path: Some(default_path),
                languages: Vec::new(),
            })
        } else {
            Ok(Self {
                config_path: None,
                languages: Vec::new(),
            })
        }
    }
}
