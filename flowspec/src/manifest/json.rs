//! JSON output formatter — produces valid JSON from manifests and diagnostics.
//!
//! Uses `serde_json::to_string_pretty()` for human-readable output that is
//! also agent-friendly. Field names match YAML output (abbreviated: vis, sig, loc).

use crate::error::ManifestError;
use crate::manifest::{DiagnosticEntry, Manifest, OutputFormatter};

/// JSON formatter for manifest and diagnostic output.
///
/// Uses serde_json for serialization. Produces pretty-printed JSON by default.
/// Field names are identical to YAML output — the abbreviated names (vis, sig, loc)
/// are baked into the struct definitions in `types.rs`.
pub struct JsonFormatter;

impl JsonFormatter {
    /// Create a new JSON formatter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsonFormatter {
    fn default() -> Self {
        Self::new()
    }
}

impl OutputFormatter for JsonFormatter {
    fn format_manifest(&self, manifest: &Manifest) -> Result<String, ManifestError> {
        serde_json::to_string_pretty(manifest).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }

    fn format_diagnostics(&self, diagnostics: &[DiagnosticEntry]) -> Result<String, ManifestError> {
        serde_json::to_string_pretty(diagnostics).map_err(|e| ManifestError::Serialization {
            reason: e.to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::types::{DiagnosticEntry, EntityEntry, EvidenceEntry, Manifest};

    #[test]
    fn json_formatter_produces_valid_json_for_full_manifest() {
        let manifest = Manifest::sample_full();
        let formatter = JsonFormatter::new();
        let result = formatter.format_manifest(&manifest);
        assert!(result.is_ok(), "Formatting failed: {:?}", result.err());

        let json = result.unwrap();
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(
            parsed.is_ok(),
            "Formatter produced invalid JSON: {:?}",
            parsed.err()
        );
    }

    #[test]
    fn json_formatter_empty_manifest() {
        let manifest = Manifest::empty();
        let formatter = JsonFormatter::new();
        let json = formatter.format_manifest(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let obj = parsed.as_object().unwrap();
        assert_eq!(
            obj.len(),
            8,
            "Empty manifest must have exactly 8 sections, got {}",
            obj.len()
        );
    }

    #[test]
    fn json_format_diagnostics_produces_array() {
        let diagnostics = vec![
            DiagnosticEntry::sample_critical(),
            DiagnosticEntry::sample_warning(),
        ];
        let formatter = JsonFormatter::new();
        let json = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let list = parsed
            .as_array()
            .expect("diagnose JSON output must be an array");
        assert_eq!(list.len(), 2);

        for entry in list {
            for field in &[
                "id",
                "pattern",
                "severity",
                "confidence",
                "entity",
                "message",
                "evidence",
                "suggestion",
                "loc",
            ] {
                assert!(
                    entry.get(field).is_some(),
                    "Diagnostic entry missing field '{}' in JSON",
                    field
                );
            }
        }
    }

    #[test]
    fn json_formatter_handles_unicode() {
        let mut manifest = Manifest::empty();
        manifest.entities.push(EntityEntry {
            id: "module::über_funktion".to_string(),
            kind: "fn".to_string(),
            vis: "pub".to_string(),
            sig: "(名前: str) -> 文字列".to_string(),
            loc: "über.py:1".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        });

        let formatter = JsonFormatter::new();
        let json = formatter.format_manifest(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let entities = parsed["entities"].as_array().unwrap();
        assert_eq!(entities[0]["id"].as_str().unwrap(), "module::über_funktion");
        assert_eq!(
            entities[0]["sig"].as_str().unwrap(),
            "(名前: str) -> 文字列"
        );
    }

    #[test]
    fn json_formatter_escapes_special_characters() {
        let mut manifest = Manifest::empty();
        manifest.metadata.project = r#"project "with" quotes & back\slashes"#.to_string();
        manifest.entities.push(EntityEntry {
            id: "module::func_with_\"quotes\"".to_string(),
            kind: "fn".to_string(),
            vis: "pub".to_string(),
            sig: "() -> str".to_string(),
            loc: r"src\path\file.py:1".to_string(),
            calls: vec![],
            called_by: vec![],
            annotations: vec![],
        });

        let formatter = JsonFormatter::new();
        let json = formatter.format_manifest(&manifest).unwrap();

        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json);
        assert!(
            parsed.is_ok(),
            "Special characters produced invalid JSON: {:?}\nJSON:\n{}",
            parsed.err(),
            &json[..json.len().min(500)]
        );

        let val = parsed.unwrap();
        assert!(val["metadata"]["project"]
            .as_str()
            .unwrap()
            .contains("quotes"));
    }

    #[test]
    fn json_evidence_none_fields_omitted_not_null() {
        let diagnostics = vec![DiagnosticEntry {
            id: "D001".to_string(),
            pattern: "test".to_string(),
            severity: "warning".to_string(),
            confidence: "high".to_string(),
            entity: "test::fn".to_string(),
            message: "test".to_string(),
            evidence: vec![EvidenceEntry {
                observation: "test observation".to_string(),
                location: None,
                context: None,
            }],
            suggestion: "test".to_string(),
            loc: "test.py:1".to_string(),
        }];

        let formatter = JsonFormatter::new();
        let json = formatter.format_diagnostics(&diagnostics).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        let evidence = &parsed[0]["evidence"][0];
        assert!(
            evidence.get("observation").is_some(),
            "observation must be present"
        );
        assert!(
            evidence.get("location").is_none(),
            "None location must be OMITTED from JSON, not present as null. Got: {:?}",
            evidence.get("location")
        );
        assert!(
            evidence.get("context").is_none(),
            "None context must be OMITTED from JSON, not present as null. Got: {:?}",
            evidence.get("context")
        );
    }

    #[test]
    fn json_serde_rename_attributes_applied() {
        let manifest = Manifest::sample_full();
        let formatter = JsonFormatter::new();
        let json = formatter.format_manifest(&manifest).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object(), "Root must be an object");

        if let Some(boundaries) = parsed["boundaries"].as_array() {
            for boundary in boundaries {
                assert!(
                    boundary.get("type").is_some(),
                    "BoundaryEntry must use 'type' not 'boundary_type' in JSON"
                );
                assert!(
                    boundary.get("boundary_type").is_none(),
                    "BoundaryEntry must not expose Rust field name 'boundary_type'"
                );
            }
        }
    }

    #[test]
    fn json_roundtrip_manifest() {
        let original = Manifest::sample_full();
        let formatter = JsonFormatter::new();
        let json = formatter.format_manifest(&original).unwrap();

        let deserialized: Manifest =
            serde_json::from_str(&json).expect("JSON output must deserialize back to Manifest");

        assert_eq!(deserialized.metadata.project, original.metadata.project);
        assert_eq!(deserialized.entities.len(), original.entities.len());
        assert_eq!(deserialized.diagnostics.len(), original.diagnostics.len());
        assert_eq!(deserialized.flows.len(), original.flows.len());
    }
}
