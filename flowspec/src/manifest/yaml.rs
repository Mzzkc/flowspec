//! YAML output formatter — produces valid YAML from manifests and diagnostics.
//!
//! Key ordering follows struct field declaration order (serde_yaml serializes
//! in declaration order): metadata → summary → diagnostics → entities → flows →
//! boundaries → dependency_graph → type_flows. Most valuable sections first.

use crate::error::ManifestError;
use crate::manifest::{DiagnosticEntry, Manifest, OutputFormatter};

/// YAML formatter for manifest and diagnostic output.
///
/// Uses serde_yaml for serialization. Struct field ordering in the types
/// module controls YAML key ordering — most valuable sections appear first.
pub struct YamlFormatter;

impl YamlFormatter {
    /// Create a new YAML formatter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for YamlFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for YamlFormatter {
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError> {
        serde_yaml::to_string(manifest).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }

    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError> {
        serde_yaml::to_string(diagnostics).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::types::{EntityEntry, Manifest};

    #[test]
    fn yaml_formatter_produces_valid_yaml_for_full_manifest() {
        let manifest = Manifest::sample_full();
        let formatter = YamlFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(result.is_ok(), "Formatting failed: {:?}", result.err());

        let yaml = result.unwrap();
        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&yaml);
        assert!(
            parsed.is_ok(),
            "Formatter produced invalid YAML: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn yaml_formatter_produces_valid_yaml_for_empty_manifest() {
        let manifest = Manifest::empty();
        let formatter = YamlFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(result.is_ok());

        let yaml = result.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();

        let map = parsed.as_mapping().unwrap();
        assert_eq!(map.len(), 8, "Empty manifest must have exactly 8 sections");
    }

    #[test]
    fn yaml_formatter_format_diagnostics_for_diagnose_command() {
        let diagnostics = vec![
            DiagnosticEntry::sample_critical(),
            DiagnosticEntry::sample_warning(),
        ];
        let formatter = YamlFormatter::new();
        let result = formatter.format_diagnostics(&diagnostics);
        assert!(result.is_ok());

        let yaml = result.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let list = parsed
            .as_sequence()
            .expect("diagnose output must be a list");
        assert_eq!(list.len(), 2);
    }

    #[test]
    fn yaml_formatter_does_not_panic_on_special_yaml_characters() {
        let mut manifest = Manifest::empty();
        manifest.metadata.project = "project: with: colons & \"quotes\"".to_string();
        let formatter = YamlFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(
            result.is_ok(),
            "Formatter panicked on special YAML characters"
        );

        let yaml = result.unwrap();
        let parsed: Result<serde_yaml::Value, _> = serde_yaml::from_str(&yaml);
        assert!(parsed.is_ok(), "Special characters produced invalid YAML");
    }

    #[test]
    fn yaml_formatter_handles_unicode_in_identifiers() {
        let mut manifest = Manifest::empty();
        manifest.entities.push(EntityEntry {
            id: "module::über_funktion".to_string(),
            kind: "fn".to_string(),
            vis: "pub".to_string(),
            sig: "(über: str) -> straße".to_string(),
            loc: "über.py:1".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        });
        let formatter = YamlFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(result.is_ok());

        let yaml = result.unwrap();
        let parsed: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let entities = parsed["entities"].as_sequence().unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0]["id"].as_str().unwrap(), "module::über_funktion");
    }

    #[test]
    fn yaml_key_ordering_matches_spec_priority() {
        let manifest = Manifest::sample_full();
        let formatter = YamlFormatter::new();
        let yaml = formatter.format_manifest(&manifest).unwrap();

        let metadata_pos = yaml.find("metadata:").expect("metadata key not found");
        let summary_pos = yaml.find("summary:").expect("summary key not found");
        let diagnostics_pos = yaml
            .find("diagnostics:")
            .expect("diagnostics key not found");

        assert!(
            metadata_pos < summary_pos,
            "metadata must come before summary in YAML output"
        );
        assert!(
            summary_pos < diagnostics_pos,
            "summary must come before diagnostics in YAML output"
        );
    }
}
