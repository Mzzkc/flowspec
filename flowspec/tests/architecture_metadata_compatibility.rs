//! Compatibility at the public composition boundary after primitive extraction.
use flowspec::registry::{LanguageDescriptor, PatternDescriptor, Registry};
use std::path::{Path, PathBuf};

#[test]
fn public_module_index_constructor_retains_exact_function_signature() {
    let constructor: fn(&Path, &[PathBuf], &Registry) -> flowspec::graph::ModuleIndex =
        flowspec::graph::ModuleIndex::new;
    let paths = vec![PathBuf::from("/project/unit.py")];
    let _index = constructor(Path::new("/project"), &paths, &Registry::default());
    let naming: fn(&[PathBuf]) -> std::collections::HashMap<String, PathBuf> =
        flowspec::build_module_map;
    assert_eq!(naming(&paths).get("unit"), Some(&paths[0]));
}

#[test]
fn lower_metadata_preserves_runtime_catalog_callbacks_and_aliases() {
    let registry = Registry::default();
    registry.validate().unwrap();
    let descriptors: &[LanguageDescriptor] = registry.languages();
    for (actual, metadata) in descriptors
        .iter()
        .zip(flowspec::language_metadata::BUILTIN_LANGUAGES)
    {
        assert_eq!(actual.id, metadata.id);
        assert_eq!(actual.grammar_version, metadata.grammar_version);
        assert_eq!(actual.aliases, metadata.aliases);
        assert_eq!(actual.extensions, metadata.extensions);
        assert_eq!(actual.factory.unwrap()().language_name(), actual.id);
        for extension in metadata.extensions {
            let path = PathBuf::from(format!("unit.{extension}"));
            assert_eq!(
                flowspec::registry::language_id_for_path(&path),
                Some(metadata.id)
            );
            assert_eq!(registry.language_for_path(&path).unwrap().id, metadata.id);
        }
        for alias in metadata.aliases {
            assert_eq!(registry.language(alias).unwrap().id, actual.id);
        }
    }
    let patterns: &[PatternDescriptor] = registry.patterns();
    assert_eq!(patterns.len(), 13);
    for (actual, metadata) in patterns.iter().zip(flowspec::catalog::PATTERN_METADATA) {
        assert_eq!(actual.pattern, metadata.pattern);
        assert_eq!(actual.pattern.name(), actual.id);
        assert_eq!(actual.id, metadata.id);
        assert_eq!(actual.dispatch_order, metadata.dispatch_order);
        assert_eq!(actual.implementation_id, metadata.implementation_id);
        assert_eq!(actual.required_capabilities, metadata.required_capabilities);
        assert_eq!(actual.applicability, metadata.applicability);
        assert!(actual.detector.is_some());
        assert_eq!(
            serde_json::to_value(actual.pattern).unwrap(),
            serde_json::to_value(metadata.pattern).unwrap()
        );
    }
    assert_eq!(
        flowspec::registry::PATTERN_IDS,
        flowspec::catalog::PATTERN_IDS
    );
}

#[test]
fn public_error_messages_preserve_canonical_names_without_registry_wiring() {
    let language = flowspec::FlowspecError::UnsupportedLanguage {
        language: "foreign".into(),
    }
    .to_string();
    assert_eq!(language, "unsupported language: foreign (fix: supported languages are python, javascript, typescript, rust)");
    let pattern = flowspec::error::FlowspecError::UnknownPattern {
        pattern: "foreign".into(),
    }
    .to_string();
    assert_eq!(
        pattern,
        format!(
            "unknown diagnostic pattern: foreign (fix: valid patterns are {})",
            flowspec::registry::PATTERN_IDS.join(", ")
        )
    );
}
