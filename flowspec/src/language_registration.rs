//! Actual adapter/recovery wiring, separate from lower language identity metadata.
use crate::parser::LanguageAdapter;
use serde_json::{json, Value};
use std::path::Path;
/// Adapter constructor used by ordinary analysis.
pub type AdapterFactory = fn() -> Box<dyn LanguageAdapter>;
/// Recovery observation in the actual grammar used by an adapter.
pub type RecoveryProbe = fn(&Path, &str) -> bool;
/// Concrete loss coordinates returned by a grammar probe.
pub type RecoveryRange = fn(&Path, &str) -> Option<(u64, u64, u64, u64)>;
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
pub(crate) fn builtin_languages() -> Vec<LanguageDescriptor> {
    vec![
        LanguageDescriptor {
            id: crate::language_metadata::BUILTIN_LANGUAGES[0].id.into(),
            aliases: crate::language_metadata::BUILTIN_LANGUAGES[0]
                .aliases
                .iter()
                .map(|s| (*s).into())
                .collect(),
            extensions: crate::language_metadata::BUILTIN_LANGUAGES[0]
                .extensions
                .iter()
                .map(|s| (*s).into())
                .collect(),
            default_enabled: true,
            implementation_id: "flowspec.adapter.python".into(),
            adapter_version: "structured-v2".into(),
            grammar_version: crate::language_metadata::BUILTIN_LANGUAGES[0]
                .grammar_version
                .into(),
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
            id: crate::language_metadata::BUILTIN_LANGUAGES[1].id.into(),
            aliases: crate::language_metadata::BUILTIN_LANGUAGES[1]
                .aliases
                .iter()
                .map(|s| (*s).into())
                .collect(),
            extensions: crate::language_metadata::BUILTIN_LANGUAGES[1]
                .extensions
                .iter()
                .map(|s| (*s).into())
                .collect(),
            default_enabled: true,
            implementation_id: "flowspec.adapter.js".into(),
            adapter_version: "structured-v2".into(),
            grammar_version: crate::language_metadata::BUILTIN_LANGUAGES[1]
                .grammar_version
                .into(),
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
            id: crate::language_metadata::BUILTIN_LANGUAGES[2].id.into(),
            aliases: crate::language_metadata::BUILTIN_LANGUAGES[2]
                .aliases
                .iter()
                .map(|s| (*s).into())
                .collect(),
            extensions: crate::language_metadata::BUILTIN_LANGUAGES[2]
                .extensions
                .iter()
                .map(|s| (*s).into())
                .collect(),
            default_enabled: true,
            implementation_id: "flowspec.adapter.typescript".into(),
            adapter_version: "structured-v2".into(),
            grammar_version: crate::language_metadata::BUILTIN_LANGUAGES[2]
                .grammar_version
                .into(),
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
            id: crate::language_metadata::BUILTIN_LANGUAGES[3].id.into(),
            aliases: crate::language_metadata::BUILTIN_LANGUAGES[3]
                .aliases
                .iter()
                .map(|s| (*s).into())
                .collect(),
            extensions: crate::language_metadata::BUILTIN_LANGUAGES[3]
                .extensions
                .iter()
                .map(|s| (*s).into())
                .collect(),
            default_enabled: true,
            implementation_id: "flowspec.adapter.rust".into(),
            adapter_version: "structured-v2".into(),
            grammar_version: crate::language_metadata::BUILTIN_LANGUAGES[3]
                .grammar_version
                .into(),
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
