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
            let config_file = read_config_file(path)?;
            return Ok(Self {
                config_path: Some(path.to_path_buf()),
                languages: config_file.languages,
                exclude: config_file.exclude,
            });
        }

        let default_path = project_root.join(".flowspec").join("config.yaml");
        if default_path.exists() {
            let config_file = read_config_file(&default_path)?;
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
fn read_config_file(path: &Path) -> Result<ConfigFile, crate::error::FlowspecError> {
    let failure = |reason: String| crate::error::FlowspecError::Config {
        reason: format!("{}: {}", path.display(), reason),
        suggestion: "Correct the config YAML or pass a valid --config path".to_string(),
    };
    let content =
        std::fs::read_to_string(path).map_err(|e| failure(format!("read failed: {e}")))?;

    // Empty content or whitespace-only → defaults
    if content.trim().is_empty() {
        return Ok(ConfigFile::default());
    }

    // Warn on unknown top-level config keys (dogfood 2026-06-15: the silent-drop of
    // nested analysis.*/diagnostics.* — which ConfigFile doesn't deserialize — caused
    // the test-pollution pain). Only `languages` + `exclude` are read; surface the rest
    // loudly so misconfigured keys (e.g. `analysis.ignore` vs top-level `exclude`) aren't
    // silently ignored.
    let value = serde_yaml::from_str::<serde_yaml::Value>(&content)
        .map_err(|e| failure(format!("parse failed: {e}")))?;
    if value.is_null() {
        return Ok(ConfigFile::default());
    }
    if let serde_yaml::Value::Mapping(map) = value {
        for key in map.keys() {
            if let Some(k) = key.as_str() {
                if k != "languages" && k != "exclude" {
                    return Err(failure(format!("unknown config key `{k}`")));
                }
            } else {
                return Err(failure("non-string config key".to_string()));
            }
        }
    }

    let config = serde_yaml::from_str::<ConfigFile>(&content)
        .map_err(|e| failure(format!("parse failed: {e}")))?;
    for pattern in &config.exclude {
        if Path::new(pattern)
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(failure(format!("exclude path escapes project: {pattern}")));
        }
    }
    Ok(config)
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
    fn test_config_load_malformed_default_yaml_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        let path = flowspec_dir.join("config.yaml");
        std::fs::write(&path, "{{{{not valid yaml!@#$").unwrap();

        match Config::load(tmp.path(), None) {
            Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                assert!(reason.contains(&path.display().to_string()), "{reason}");
                assert!(
                    reason.to_lowercase().contains("parse")
                        || reason.to_lowercase().contains("yaml")
                        || reason.to_lowercase().contains("syntax"),
                    "missing parse cause: {reason}"
                );
                assert!(!suggestion.trim().is_empty(), "missing fix suggestion");
            }
            other => panic!("present malformed default config must fail: {other:?}"),
        }
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
    fn test_config_load_unknown_fields_error() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        let path = flowspec_dir.join("config.yaml");

        let mut accepted_unknown_keys = Vec::new();
        for (key, yaml) in [
            ("future_field", "languages: [rust]\nfuture_field: value\n"),
            (
                "diagnostics",
                "languages: [rust]\ndiagnostics:\n  suppressions: []\n",
            ),
            ("ignore", "languages: [rust]\nignore: [vendor/]\n"),
        ] {
            std::fs::write(&path, yaml).unwrap();
            match Config::load(tmp.path(), None) {
                Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                    assert!(
                        reason.contains(&path.display().to_string()),
                        "{key}: {reason}"
                    );
                    assert!(reason.contains(key), "missing rejected key {key}: {reason}");
                    assert!(!suggestion.trim().is_empty(), "missing fix for {key}");
                }
                other => accepted_unknown_keys.push(format!("{key} returned {other:?}")),
            }
        }
        assert!(
            accepted_unknown_keys.is_empty(),
            "unknown keys must fail separately: {}",
            accepted_unknown_keys.join("; ")
        );
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
    fn test_config_path_yaml_key_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        let path = flowspec_dir.join("config.yaml");
        std::fs::write(&path, "config_path: /etc/evil\nlanguages: []\n").unwrap();

        match Config::load(tmp.path(), None) {
            Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                assert!(reason.contains(&path.display().to_string()), "{reason}");
                assert!(reason.contains("config_path"), "{reason}");
                assert!(!suggestion.trim().is_empty(), "missing fix suggestion");
            }
            other => panic!("YAML config_path must be rejected: {other:?}"),
        }
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
    fn test_config_directory_errors_without_panic() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        let path = flowspec_dir.join("config.yaml");
        std::fs::create_dir_all(&path).unwrap();

        let result = std::panic::catch_unwind(|| Config::load(tmp.path(), None));
        assert!(
            result.is_ok(),
            "directory read must return an error, not panic"
        );
        match result.unwrap() {
            Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                assert!(reason.contains(&path.display().to_string()), "{reason}");
                assert!(
                    reason.to_lowercase().contains("read")
                        || reason.to_lowercase().contains("directory"),
                    "missing read cause: {reason}"
                );
                assert!(!suggestion.trim().is_empty(), "missing fix suggestion");
            }
            other => panic!("directory config must fail: {other:?}"),
        }
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
        let path = flowspec_dir.join("config.yaml");
        std::fs::write(&path, "languages: &values [python]\nexclude: *values\n").unwrap();

        let config = Config::load(tmp.path(), None).unwrap();
        assert_eq!(config.config_path, Some(path.clone()));
        assert_eq!(config.languages, vec!["python".to_string()]);
        assert_eq!(config.exclude, vec!["python".to_string()]);

        std::fs::write(&path, "defaults: &values [python]\nlanguages: *values\n").unwrap();
        match Config::load(tmp.path(), None) {
            Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                assert!(reason.contains(&path.display().to_string()), "{reason}");
                assert!(reason.contains("defaults"), "{reason}");
                assert!(!suggestion.trim().is_empty(), "missing fix suggestion");
            }
            other => panic!("anchor on unknown defaults key must fail: {other:?}"),
        }
    }

    #[test]
    fn test_config_malformed_yaml_explicit_path_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let flowspec_dir = tmp.path().join(".flowspec");
        std::fs::create_dir_all(&flowspec_dir).unwrap();
        std::fs::write(flowspec_dir.join("config.yaml"), "languages: [python]\n").unwrap();
        let custom = tmp.path().join("bad.yaml");
        std::fs::write(&custom, "{{{{invalid!").unwrap();

        match Config::load(tmp.path(), Some(&custom)) {
            Err(crate::error::FlowspecError::Config { reason, suggestion }) => {
                assert!(reason.contains(&custom.display().to_string()), "{reason}");
                assert!(
                    reason.to_lowercase().contains("parse")
                        || reason.to_lowercase().contains("yaml")
                        || reason.to_lowercase().contains("syntax"),
                    "missing parse cause: {reason}"
                );
                assert!(!suggestion.trim().is_empty(), "missing fix suggestion");
            }
            other => panic!("invalid explicit config must not use valid implicit: {other:?}"),
        }
    }
}
