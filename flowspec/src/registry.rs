//! Authoritative language and diagnostic registry. Callbacks are runtime authority;
//! explicit implementation IDs and versions bind them in deterministic metadata.
use crate::analyzer::diagnostic::{Diagnostic, DiagnosticPattern};
use crate::analyzer::patterns;
use crate::parser::LanguageAdapter;
use crate::{FlowspecError, Graph};
use serde_json::{json, Value};
use std::path::Path;

/// Adapter constructor used by ordinary analysis.
pub type AdapterFactory = fn() -> Box<dyn LanguageAdapter>;
/// Recovery observation in the actual grammar used by an adapter.
pub type RecoveryProbe = fn(&Path, &str) -> bool;
/// Concrete loss coordinates returned by a grammar probe.
pub type RecoveryRange = fn(&Path, &str) -> Option<(u64, u64, u64, u64)>;
/// Production detector callback.
pub type Detector = fn(&Graph, &Path) -> Vec<Diagnostic>;

/// An isolated adapter registration. Extensions omit dots. New languages must
/// remain opt-in; registration does not qualify their semantics for launch.
#[derive(Clone)]
pub struct LanguageDescriptor {
    pub id: String,
    pub aliases: Vec<String>,
    pub extensions: Vec<String>,
    pub default_enabled: bool,
    pub implementation_id: String,
    pub adapter_version: String,
    pub grammar_version: String,
    pub capabilities: Vec<String>,
    pub limitations: Vec<String>,
    pub factory: Option<AdapterFactory>,
    pub recovery_id: String,
    pub recovery: Option<RecoveryProbe>,
    pub recovery_range: Option<RecoveryRange>,
}
impl LanguageDescriptor {
    /// Result-determining data only; no pointer addresses or map iteration order.
    pub fn metadata(&self) -> Value {
        json!({"id":self.id,"aliases":self.aliases,"extensions":self.extensions,
        "default_enabled":self.default_enabled,"implementation_id":self.implementation_id,
        "adapter_version":self.adapter_version,"grammar_version":self.grammar_version,
        "capabilities":self.capabilities,"limitations":self.limitations,"recovery_id":self.recovery_id,
        "recovery_available":self.recovery.is_some(),"recovery_range_available":self.recovery_range.is_some()})
    }
}

/// Stable catalog order and preserved historical dispatch order are distinct.
#[derive(Clone)]
pub struct PatternDescriptor {
    pub pattern: DiagnosticPattern,
    pub id: &'static str,
    pub dispatch_order: usize,
    pub implementation_id: &'static str,
    pub required_capabilities: &'static [&'static str],
    pub applicability: &'static str,
    pub detector: Option<Detector>,
}
impl PatternDescriptor {
    pub fn metadata(&self) -> Value {
        json!({"id":self.id,"dispatch_order":self.dispatch_order,"implementation_id":self.implementation_id,
        "required_capabilities":self.required_capabilities,"applicability":self.applicability,"default_enabled":true,
        "detector_version":env!("CARGO_PKG_VERSION")})
    }
}

macro_rules! diagnostic_catalog {
    ($(($variant:ident, $id:literal, $module:ident, $order:literal, [$($cap:literal),*], $app:literal)),*) => {
        /// Canonical accepted catalog order.
        pub const PATTERN_IDS: [&str;13] = [$($id),*];
        /// Canonical descriptors bind each name to its actual production detector.
        pub fn pattern_descriptors() -> Vec<PatternDescriptor> { vec![$(PatternDescriptor {
            pattern:DiagnosticPattern::$variant, id:$id, dispatch_order:$order,
            implementation_id:concat!("flowspec.detector.", $id), required_capabilities:&[$($cap),*],
            applicability:$app, detector:Some(patterns::$module::detect)
        }),*] }
    }
}
diagnostic_catalog!(
    (
        IsolatedCluster,
        "isolated_cluster",
        isolated_cluster,
        0,
        ["symbols", "calls", "resolution"],
        "selected_language"
    ),
    (
        DataDeadEnd,
        "data_dead_end",
        data_dead_end,
        1,
        [
            "symbols",
            "calls",
            "value_references",
            "exports",
            "registration",
            "consumer_closure"
        ],
        "selected_language"
    ),
    (
        PartialWiring,
        "partial_wiring",
        partial_wiring,
        10,
        ["symbols", "calls", "control_flow"],
        "selected_language"
    ),
    (
        OrphanedImplementation,
        "orphaned_impl",
        orphaned_implementation,
        4,
        ["symbols", "calls", "resolution", "types", "implementations"],
        "selected_language"
    ),
    (
        Duplication,
        "duplication",
        duplication,
        11,
        ["symbols", "binding_structure"],
        "selected_language"
    ),
    (
        ContractMismatch,
        "contract_mismatch",
        contract_mismatch,
        8,
        ["symbols", "calls", "control_flow", "types"],
        "selected_language"
    ),
    (
        CircularDependency,
        "circular_dependency",
        circular_dependency,
        3,
        ["imports", "dependency_edges"],
        "selected_language"
    ),
    (
        LayerViolation,
        "layer_violation",
        layer_violation,
        6,
        ["imports", "dependency_edges"],
        "explicit_layer_policy"
    ),
    (
        IncompleteMigration,
        "incomplete_migration",
        incomplete_migration,
        9,
        ["imports", "resolution"],
        "selected_language"
    ),
    (
        AsymmetricHandling,
        "asymmetric_handling",
        asymmetric_handling,
        12,
        ["symbols", "calls", "control_flow"],
        "selected_language"
    ),
    (
        StaleReference,
        "stale_reference",
        stale_reference,
        7,
        ["imports", "resolution"],
        "selected_language"
    ),
    (
        PhantomDependency,
        "phantom_dependency",
        phantom_dependency,
        2,
        ["imports", "exports", "value_references"],
        "selected_language"
    ),
    (
        MissingReexport,
        "missing_reexport",
        missing_reexport,
        5,
        ["imports", "exports", "value_references"],
        "selected_language"
    )
);

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
        crate::report_authority::hash("registry", &self.metadata())
    }
}
fn registry_error(reason: &str) -> FlowspecError {
    FlowspecError::Config {
        reason: format!("registry: {reason}"),
        suggestion: "provide a unique complete registration".into(),
    }
}
/// Canonical default names, used by help and error rendering.
pub fn language_names() -> String {
    default_registry()
        .languages()
        .iter()
        .map(|d| d.id.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}
pub fn pattern_names() -> String {
    PATTERN_IDS.join(", ")
}
/// Default classification; extension-aware callers with custom registries use their instance.
pub fn language_id_for_path(path: &Path) -> Option<&'static str> {
    default_registry()
        .language_for_path(path)
        .map(|d| d.id.as_str())
}
fn python_factory() -> Box<dyn LanguageAdapter> {
    Box::new(crate::parser::python::PythonAdapter::new())
}
fn js_factory() -> Box<dyn LanguageAdapter> {
    Box::new(crate::parser::javascript::JsAdapter::new())
}
fn ts_factory() -> Box<dyn LanguageAdapter> {
    Box::new(crate::parser::typescript::TsAdapter::new())
}
fn rust_factory() -> Box<dyn LanguageAdapter> {
    Box::new(crate::parser::rust::RustAdapter::new())
}
fn builtin_languages() -> Vec<LanguageDescriptor> {
    vec![
        LanguageDescriptor {
            id: "python".into(),
            aliases: vec!["py".into()],
            extensions: vec!["py".into()],
            default_enabled: true,
            implementation_id: "flowspec.adapter.python".into(),
            adapter_version: "structured-v1".into(),
            grammar_version: "tree-sitter-python/0.25.0".into(),
            capabilities: crate::parser::report::implemented_capabilities("python")
                .iter()
                .map(|c| c.as_str().into())
                .collect(),
            limitations: vec![],
            factory: Some(python_factory),
            recovery_id: "flowspec.recovery.python".into(),
            recovery: Some(crate::parser::source_has_parse_loss),
            recovery_range: Some(crate::parser::source_parse_loss_range),
        },
        LanguageDescriptor {
            id: "javascript".into(),
            aliases: vec!["js".into()],
            extensions: vec!["js".into(), "jsx".into(), "mjs".into(), "cjs".into()],
            default_enabled: true,
            implementation_id: "flowspec.adapter.js".into(),
            adapter_version: "structured-v1".into(),
            grammar_version: "tree-sitter-javascript/0.25.0".into(),
            capabilities: crate::parser::report::implemented_capabilities("javascript")
                .iter()
                .map(|c| c.as_str().into())
                .collect(),
            limitations: vec![],
            factory: Some(js_factory),
            recovery_id: "flowspec.recovery.javascript".into(),
            recovery: Some(crate::parser::source_has_parse_loss),
            recovery_range: Some(crate::parser::source_parse_loss_range),
        },
        LanguageDescriptor {
            id: "typescript".into(),
            aliases: vec!["ts".into()],
            extensions: vec!["ts".into(), "tsx".into()],
            default_enabled: true,
            implementation_id: "flowspec.adapter.typescript".into(),
            adapter_version: "structured-v1".into(),
            grammar_version: "tree-sitter-typescript/0.23.2+tsx".into(),
            capabilities: crate::parser::report::implemented_capabilities("typescript")
                .iter()
                .map(|c| c.as_str().into())
                .collect(),
            limitations: vec!["advanced_type_evaluation_unavailable".into()],
            factory: Some(ts_factory),
            recovery_id: "flowspec.recovery.typescript".into(),
            recovery: Some(crate::parser::source_has_parse_loss),
            recovery_range: Some(crate::parser::source_parse_loss_range),
        },
        LanguageDescriptor {
            id: "rust".into(),
            aliases: vec!["rs".into()],
            extensions: vec!["rs".into()],
            default_enabled: true,
            implementation_id: "flowspec.adapter.rust".into(),
            adapter_version: "structured-v1".into(),
            grammar_version: "tree-sitter-rust/0.24.0".into(),
            capabilities: crate::parser::report::implemented_capabilities("rust")
                .iter()
                .map(|c| c.as_str().into())
                .collect(),
            limitations: vec![],
            factory: Some(rust_factory),
            recovery_id: "flowspec.recovery.rust".into(),
            recovery: Some(crate::parser::source_has_parse_loss),
            recovery_range: Some(crate::parser::source_parse_loss_range),
        },
    ]
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
