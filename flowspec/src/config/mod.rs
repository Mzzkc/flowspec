//! Configuration loading and validation.
//!
//! Loads project configuration from `.flowspec/config.yaml` if it exists,
//! otherwise uses defaults. Supports explicit config path override via
//! the `--config` CLI flag. Currently handles language selection; layer
//! violation rules and other analysis settings are planned for v0.2.

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_load_no_config_file_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::load(tmp.path(), None).unwrap();

        assert!(
            config.config_path.is_none(),
            "No config file → config_path should be None"
        );
        assert!(
            config.languages.is_empty(),
            "Default config should have empty languages"
        );
    }

    #[test]
    fn test_config_load_nonexistent_explicit_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let bad_path = tmp.path().join("nonexistent.yaml");

        let result = Config::load(tmp.path(), Some(&bad_path));
        assert!(
            result.is_err(),
            "Loading nonexistent explicit config must fail"
        );

        let err_msg = format!("{}", result.unwrap_err());
        assert!(
            err_msg.contains("not found"),
            "Error message must mention 'not found', got: {}",
            err_msg
        );
    }

    #[test]
    fn test_config_load_with_existing_config_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(flowspec_dir.join("config.yaml"), "languages: [python]").unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert!(
            config.config_path.is_some(),
            "Existing .flowspec/config.yaml must be found"
        );
    }

    #[test]
    fn test_config_load_explicit_path_overrides_default() {
        let tmp = tempfile::tempdir().unwrap();
        let custom_config = tmp.path().join("custom.yaml");
        std::fs::write(&custom_config, "languages: [rust]").unwrap();

        let config = Config::load(tmp.path(), Some(&custom_config)).unwrap();
        assert_eq!(
            config.config_path,
            Some(custom_config),
            "Explicit path must be used over default"
        );
    }
}
