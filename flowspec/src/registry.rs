//! Validated runtime registry: joins lower identities to actual callback wiring.
pub use crate::catalog::{pattern_names, PATTERN_IDS};
pub use crate::detector_registration::{pattern_descriptors, Detector, PatternDescriptor};
use crate::error::FlowspecError;
use crate::language_metadata::LanguageLookup;
pub use crate::language_metadata::{language_id_for_path, language_names};
use crate::language_registration::builtin_languages;
pub use crate::language_registration::{
    AdapterFactory, LanguageDescriptor, RecoveryProbe, RecoveryRange,
};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
/// Validated, instance-local registry. No global mutable registration state.
#[derive(Clone)]
pub struct Registry {
    languages: Vec<LanguageDescriptor>,
    patterns: Vec<PatternDescriptor>,
}
impl Default for Registry {
    fn default() -> Self {
        Self {
            languages: builtin_languages(),
            patterns: pattern_descriptors(),
        }
    }
}
impl Registry {
    pub fn languages(&self) -> &[LanguageDescriptor] {
        &self.languages
    }
    pub fn patterns(&self) -> &[PatternDescriptor] {
        &self.patterns
    }
    pub fn language(&self, id: &str) -> Option<&LanguageDescriptor> {
        self.languages
            .iter()
            .find(|d| d.id == id || d.aliases.iter().any(|a| a == id))
    }
    pub fn language_for_path(&self, path: &Path) -> Option<&LanguageDescriptor> {
        let ext = path.extension()?.to_str()?;
        self.languages
            .iter()
            .find(|d| d.extensions.iter().any(|e| e == ext))
    }
    pub fn register_language(
        &mut self,
        descriptor: LanguageDescriptor,
    ) -> Result<(), FlowspecError> {
        if descriptor.default_enabled {
            return Err(registry_error(
                "extension languages must be disabled by default",
            ));
        }
        let mut candidate = self.clone();
        candidate.languages.push(descriptor);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Construct explicit catalogs, rejecting missing/duplicate callback mappings.
    pub fn new(
        languages: Vec<LanguageDescriptor>,
        patterns: Vec<PatternDescriptor>,
    ) -> Result<Self, FlowspecError> {
        let registry = Self {
            languages,
            patterns,
        };
        registry.validate()?;
        Ok(registry)
    }
    pub fn validate(&self) -> Result<(), FlowspecError> {
        let defaults: Vec<_> = self
            .languages
            .iter()
            .filter(|d| d.default_enabled)
            .map(|d| d.id.as_str())
            .collect();
        if defaults
            != builtin_languages()
                .iter()
                .map(|d| d.id.as_str())
                .collect::<Vec<_>>()
        {
            return Err(registry_error(
                "exactly four canonical default languages required",
            ));
        }
        let mut names = std::collections::HashSet::new();
        let mut extensions = std::collections::HashSet::new();
        for d in &self.languages {
            if d.factory.is_none()
                || d.id.is_empty()
                || d.implementation_id.is_empty()
                || d.adapter_version.is_empty()
                || d.grammar_version.is_empty()
                || d.extensions.is_empty()
                || d.recovery_id.is_empty()
            {
                return Err(registry_error(
                    "missing language factory or identity metadata",
                ));
            }
            if let Some(expected) = builtin_languages()
                .iter()
                .find(|expected| expected.id == d.id)
            {
                if !std::ptr::fn_addr_eq(
                    d.factory.expect("present factory"),
                    expected.factory.expect("builtin factory"),
                ) {
                    return Err(registry_error("mismatched built-in adapter factory"));
                }
            } else if (d.factory.expect("present factory"))().language_name() != d.id {
                return Err(registry_error("mismatched extension adapter identity"));
            }
            if d.capabilities
                .iter()
                .any(|cap| !SUPPORTED_CAPABILITIES.contains(&cap.as_str()))
            {
                return Err(registry_error("unsupported capability declaration"));
            }
            for id in std::iter::once(&d.id).chain(d.aliases.iter()) {
                if id.is_empty() || !names.insert(id) {
                    return Err(registry_error("duplicate language ID/alias"));
                }
            }
            for ext in &d.extensions {
                if ext.is_empty() || ext.contains('.') || !extensions.insert(ext) {
                    return Err(registry_error("duplicate/invalid language extension"));
                }
            }
        }
        if self.patterns.len() != PATTERN_IDS.len() {
            return Err(registry_error("missing canonical diagnostic"));
        }
        let mut order = std::collections::HashSet::new();
        for (i, p) in self.patterns.iter().enumerate() {
            if p.id != PATTERN_IDS[i]
                || p.detector.is_none()
                || !order.insert(p.dispatch_order)
                || p.dispatch_order >= 13
                || p.pattern != pattern_descriptors()[i].pattern
                || !p.detector.is_some_and(|detector| {
                    std::ptr::fn_addr_eq(
                        detector,
                        pattern_descriptors()[i].detector.expect("builtin detector"),
                    )
                })
            {
                return Err(registry_error(
                    "duplicate/mismatched diagnostic or missing detector",
                ));
            }
        }
        Ok(())
    }
    pub fn metadata(&self) -> Value {
        json!({"languages":self.languages.iter().map(LanguageDescriptor::metadata).collect::<Vec<_>>(),"patterns":self.patterns.iter().map(PatternDescriptor::metadata).collect::<Vec<_>>()})
    }
    /// Preserve the accepted bounded report contract. Full descriptors are
    /// exposed by metadata() and the ordinary CLI registry catalog.
    pub fn report_metadata(&self) -> Value {
        json!({"digest":self.digest()})
    }
    pub fn digest(&self) -> String {
        crate::identity::hash("registry", &self.metadata())
    }
}
fn registry_error(reason: &str) -> FlowspecError {
    FlowspecError::Config {
        reason: format!("registry: {reason}"),
        suggestion: "provide a unique complete registration".into(),
    }
}
/// Read-only built-in classification authority, shared by parser and detector callers.
pub fn default_registry() -> &'static Registry {
    static REGISTRY: std::sync::LazyLock<Registry> = std::sync::LazyLock::new(Registry::default);
    &REGISTRY
}

/// Vocabulary for declared adapter evidence; availability is per adapter.
pub const SUPPORTED_CAPABILITIES: &[&str] = &[
    "symbols",
    "scopes",
    "visibility",
    "calls",
    "call_arity",
    "imports",
    "exports",
    "reexports",
    "side_effect_imports",
    "name_use",
    "types",
    "implementations",
    "annotations",
    "dispatch",
    "binding_structure",
    "control_flow",
    "exceptional_paths",
    "cleanup",
    "returns",
    "module_boundaries",
    "architecture_layers",
    "value_references",
    "registration",
    "consumer_closure",
    "dependency_edges",
    "resolution",
];

impl LanguageLookup for Registry {
    fn language_id_for_path(&self, path: &Path) -> Option<&str> {
        self.language_for_path(path).map(|d| d.id.as_str())
    }
}
// The preserved public constructor lives at the runtime composition boundary.
// ModuleIndex itself depends only on lower language lookup and naming primitives.
impl crate::graph::ModuleIndex {
    pub fn new(root: &Path, files: &[PathBuf], registry: &Registry) -> Self {
        Self::from_language_metadata(root, files, registry)
    }
}
