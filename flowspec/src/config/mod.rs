//! Configuration loading and validation.
//!
//! Loads project configuration from `.flowspec/config.yaml` if it exists,
//! otherwise uses defaults. Supports explicit config path override via
//! the `--config` CLI flag. Handles language selection and file exclusion
//! patterns. Layer violation rules and other analysis settings are planned
//! for v0.2.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Raw YAML structure for deserialization.
///
/// This intermediate struct avoids exposing `config_path` to serde
/// (it must come from the file's actual location, not from YAML content).
#[derive(Debug, Clone, Deserialize, Default)]
struct ConfigFile {
    /// Languages to analyze (empty = auto-detect).
    #[serde(default)]
    languages: Vec<String>,
    /// Patterns to exclude from analysis.
    #[serde(default)]
    exclude: Vec<String>,
}

/// Project configuration loaded from `.flowspec/config.yaml`.
#[derive(Debug, Clone, Default)]
pub struct Config {
    /// Path to the config file, if one was found.
    pub config_path: Option<PathBuf>,
    /// Languages to analyze (empty = auto-detect).
    pub languages: Vec<String>,
    /// Patterns to exclude from analysis.
    pub exclude: Vec<String>,
}

impl Config {
    /// Load configuration from the default location or a specified path.
    ///
    /// If `config_path` is Some, load from that path. Otherwise, look for
    /// `.flowspec/config.yaml` in the project root. If no config file exists,
    /// use defaults. Malformed YAML degrades gracefully to defaults with a
    /// warning rather than crashing analysis.
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
            let config_file = read_config_file(path);
            return Ok(Self {
                config_path: Some(path.to_path_buf()),
                languages: config_file.languages,
                exclude: config_file.exclude,
            });
        }

        let default_path = project_root.join(".flowspec").join("config.yaml");
        if default_path.exists() {
            let config_file = read_config_file(&default_path);
            Ok(Self {
                config_path: Some(default_path),
                languages: config_file.languages,
                exclude: config_file.exclude,
            })
        } else {
            Ok(Self {
                config_path: None,
                languages: Vec::new(),
                exclude: Vec::new(),
            })
        }
    }
}

/// Read and deserialize a config file, returning defaults on any error.
///
/// Handles: empty files, comment-only YAML, malformed YAML, encoding issues.
/// Logs a warning on parse failure rather than crashing.
fn read_config_file(path: &Path) -> ConfigFile {
    let content = match std::fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(
                "Failed to read config file {}: {} — using defaults",
                path.display(),
                e
            );
            return ConfigFile::default();
        }
    };

    // Empty content or whitespace-only → defaults
    if content.trim().is_empty() {
        return ConfigFile::default();
    }

    match serde_yaml::from_str::<ConfigFile>(&content) {
        Ok(config) => config,
        Err(e) => {
            tracing::warn!(
                "Failed to parse config file {}: {} — check YAML syntax, using defaults",
                path.display(),
                e
            );
            ConfigFile::default()
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
        assert!(
            config.exclude.is_empty(),
            "Default config should have empty exclude"
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

    // === QA-3 Category 1: Config Deserialization ===

    #[test]
    fn test_config_load_populates_languages_from_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "languages:\n  - python\n  - rust\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_eq!(
            config.languages,
            vec!["python".to_string(), "rust".to_string()],
            "Config::load() must deserialize languages from YAML"
        );
    }

    #[test]
    fn test_config_load_populates_exclude_from_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "exclude:\n  - \"target/\"\n  - \"archive/\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_eq!(
            config.exclude,
            vec!["target/".to_string(), "archive/".to_string()],
            "Config must have exclude field populated from YAML"
        );
    }

    #[test]
    fn test_config_load_explicit_path_deserializes_content() {
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("custom.yaml");
        std::fs::write(
            &custom,
            "languages:\n  - javascript\nexclude:\n  - \"vendor/\"\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), Some(&custom)).unwrap();
        assert_eq!(config.languages, vec!["javascript".to_string()]);
        assert_eq!(config.exclude, vec!["vendor/".to_string()]);
        assert_eq!(config.config_path, Some(custom));
    }

    #[test]
    fn test_config_load_malformed_yaml_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(flowspec_dir.join("config.yaml"), "{{{{not valid yaml!@#$").unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert!(
            config.languages.is_empty(),
            "Malformed YAML → default empty languages"
        );
        assert!(
            config.config_path.is_some(),
            "config_path still set even with malformed content"
        );
    }

    #[test]
    fn test_config_load_empty_file_returns_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(flowspec_dir.join("config.yaml"), "").unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert!(config.languages.is_empty());
        assert!(config.exclude.is_empty());
    }

    #[test]
    fn test_config_load_partial_config_missing_fields() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(flowspec_dir.join("config.yaml"), "languages:\n  - python\n").unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_eq!(config.languages, vec!["python".to_string()]);
        assert!(
            config.exclude.is_empty(),
            "Missing exclude field must default to empty Vec"
        );
    }

    #[test]
    fn test_config_load_unknown_fields_ignored() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "languages:\n  - rust\nfuture_field: value\ndiagnostics:\n  suppressions: []\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_eq!(config.languages, vec!["rust".to_string()]);
    }

    #[test]
    fn test_config_round_trip_init_template() {
        use crate::commands::generate_config_yaml;

        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();

        let template = generate_config_yaml(&["python".to_string(), "javascript".to_string()]);
        std::fs::write(flowspec_dir.join("config.yaml"), &template).unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert!(config.languages.contains(&"python".to_string()));
        assert!(config.languages.contains(&"javascript".to_string()));
        assert!(config.exclude.contains(&"target/".to_string()));
        assert!(config.exclude.contains(&"node_modules/".to_string()));
    }

    #[test]
    fn test_config_path_not_from_yaml() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "config_path: /etc/evil\nlanguages: []\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_ne!(
            config.config_path,
            Some(std::path::PathBuf::from("/etc/evil")),
            "config_path must not be deserialized from YAML"
        );
    }

    #[test]
    fn test_config_load_no_file_backward_compatible() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config::load(tmp.path(), None).unwrap();

        assert!(config.config_path.is_none());
        assert!(config.languages.is_empty());
        assert!(config.exclude.is_empty());
    }

    // === QA-3 Category 6: Error Handling & Edge Cases ===

    #[test]
    fn test_config_file_is_directory() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        // Create config.yaml as a directory instead of a file
        std::fs::create_dir_all(flowspec_dir.join("config.yaml")).unwrap();

        // Must NOT panic — returns defaults
        let config = Config::load(tmp.path(), None).unwrap();
        assert!(config.config_path.is_some());
        assert!(config.languages.is_empty());
    }

    #[test]
    fn test_config_yaml_only_comments() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "# This is a comment-only config\n# languages:\n#   - python\n",
        )
        .unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert!(config.languages.is_empty(), "Comment-only YAML → defaults");
    }

    #[test]
    fn test_config_yaml_anchors_and_aliases() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(
            flowspec_dir.join("config.yaml"),
            "defaults: &defaults\n  - python\nlanguages: *defaults\n",
        )
        .unwrap();

        // Either parses correctly or degrades to defaults — must not crash
        let config = Config::load(tmp.path(), None).unwrap();
        // YAML anchors: serde_yaml supports them, so languages should be ["python"]
        // But if it fails, that's also acceptable — just no crash
        let _ = config.languages;
    }

    #[test]
    fn test_config_malformed_yaml_explicit_path() {
        let tmp = tempfile::tempdir().unwrap();
        let custom = tmp.path().join("bad.yaml");
        std::fs::write(&custom, "{{{{invalid!").unwrap();

        let config = Config::load(tmp.path(), Some(&custom)).unwrap();
        assert!(
            config.languages.is_empty(),
            "Malformed YAML on explicit path → defaults"
        );
        assert!(
            config.config_path.is_some(),
            "config_path still set for explicit path"
        );
    }
}
