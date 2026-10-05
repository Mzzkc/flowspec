//! Structured parser evidence. Legacy IR alone never certifies completeness.
use super::ir::{Location, ParseResult};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ops::{Deref, DerefMut},
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Symbols,
    Scopes,
    Visibility,
    Calls,
    CallArity,
    Imports,
    Exports,
    Reexports,
    SideEffectImports,
    NameUse,
    Types,
    Implementations,
    Annotations,
    Dispatch,
    BindingStructure,
    ControlFlow,
    ExceptionalPaths,
    Cleanup,
    Returns,
    ModuleBoundaries,
    ArchitectureLayers,
    ValueReferences,
    Registration,
    ConsumerClosure,
    DependencyEdges,
    Resolution,
}
impl Capability {
    pub const ALL: &'static [Self] = &[
        Self::Symbols,
        Self::Scopes,
        Self::Visibility,
        Self::Calls,
        Self::CallArity,
        Self::Imports,
        Self::Exports,
        Self::Reexports,
        Self::SideEffectImports,
        Self::NameUse,
        Self::Types,
        Self::Implementations,
        Self::Annotations,
        Self::Dispatch,
        Self::BindingStructure,
        Self::ControlFlow,
        Self::ExceptionalPaths,
        Self::Cleanup,
        Self::Returns,
        Self::ModuleBoundaries,
        Self::ArchitectureLayers,
        Self::ValueReferences,
        Self::Registration,
        Self::ConsumerClosure,
        Self::DependencyEdges,
        Self::Resolution,
    ];
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Symbols => "symbols",
            Self::Scopes => "scopes",
            Self::Visibility => "visibility",
            Self::Calls => "calls",
            Self::CallArity => "call_arity",
            Self::Imports => "imports",
            Self::Exports => "exports",
            Self::Reexports => "reexports",
            Self::SideEffectImports => "side_effect_imports",
            Self::NameUse => "name_use",
            Self::Types => "types",
            Self::Implementations => "implementations",
            Self::Annotations => "annotations",
            Self::Dispatch => "dispatch",
            Self::BindingStructure => "binding_structure",
            Self::ControlFlow => "control_flow",
            Self::ExceptionalPaths => "exceptional_paths",
            Self::Cleanup => "cleanup",
            Self::Returns => "returns",
            Self::ModuleBoundaries => "module_boundaries",
            Self::ArchitectureLayers => "architecture_layers",
            Self::ValueReferences => "value_references",
            Self::Registration => "registration",
            Self::ConsumerClosure => "consumer_closure",
            Self::DependencyEdges => "dependency_edges",
            Self::Resolution => "resolution",
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityDisposition {
    Implemented,
    Unknown,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ParseEventCode {
    ParserRecovery,
    MissingNode,
    DepthTruncated,
    UnsupportedDynamic,
    UnsupportedSyntax,
    CapabilityUnavailable,
    UnresolvedReference,
    UnknownLegacyReport,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParseEvent {
    pub code: ParseEventCode,
    pub range: Location,
    pub byte_start: u64,
    pub byte_end: u64,
    pub affects: BTreeSet<Capability>,
    pub detail: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseReport {
    pub schema_version: u32,
    pub file: PathBuf,
    pub source_bytes: u64,
    pub source_sha256: String,
    pub language: String,
    pub adapter_version: String,
    pub grammar_version: String,
    pub capabilities: BTreeMap<Capability, CapabilityDisposition>,
    pub events: Vec<ParseEvent>,
}
impl ParseReport {
    pub fn complete_for(&self, required: &[Capability]) -> bool {
        self.schema_version == 1
            && required.iter().all(|c| {
                self.capabilities.get(c) == Some(&CapabilityDisposition::Implemented)
                    && !self.events.iter().any(|e| e.affects.contains(c))
            })
    }
    pub(crate) fn new(path: &Path, source: &str, language: &str, grammar: &str) -> Self {
        let implemented = implemented_capabilities(language);
        Self {
            schema_version: 1,
            file: path.to_path_buf(),
            source_bytes: source.len() as u64,
            source_sha256: format!("{:x}", Sha256::digest(source.as_bytes())),
            language: language.into(),
            adapter_version: "structured-v2".into(),
            grammar_version: grammar.into(),
            capabilities: Capability::ALL
                .iter()
                .map(|c| {
                    (
                        *c,
                        if implemented.contains(c) {
                            CapabilityDisposition::Implemented
                        } else {
                            CapabilityDisposition::Unknown
                        },
                    )
                })
                .collect(),
            events: vec![],
        }
    }
    pub(crate) fn event(
        &mut self,
        node: tree_sitter::Node<'_>,
        code: ParseEventCode,
        affects: &[Capability],
        detail: &str,
    ) {
        let start = node.start_position();
        let end = node.end_position();
        let event = ParseEvent {
            code,
            range: Location {
                file: self.file.clone(),
                line: start.row as u32 + 1,
                column: start.column as u32 + 1,
                end_line: end.row as u32 + 1,
                end_column: end.column as u32 + 1,
            },
            byte_start: node.start_byte() as u64,
            byte_end: node.end_byte() as u64,
            affects: affects.iter().copied().collect(),
            detail: detail.into(),
        };
        if !self.events.contains(&event) {
            self.events.push(event);
        }
    }
    pub(crate) fn finish(&mut self) {
        self.events.sort_by(|a, b| {
            (a.byte_start, a.byte_end, a.code, &a.detail).cmp(&(
                b.byte_start,
                b.byte_end,
                b.code,
                &b.detail,
            ))
        });
    }
}
/// Evidence represented by the extractors; richer flow/policy proof remains unknown.
pub fn implemented_capabilities(language: &str) -> Vec<Capability> {
    use Capability::*;
    let mut caps = vec![
        Symbols,
        Scopes,
        Visibility,
        Calls,
        CallArity,
        Imports,
        Exports,
        Reexports,
        NameUse,
        ValueReferences,
        ConsumerClosure,
        DependencyEdges,
        Resolution,
        Registration,
        BindingStructure,
    ];
    if matches!(language, "python" | "rust" | "typescript") {
        caps.extend([Types, Annotations]);
    }
    if matches!(language, "rust" | "typescript") {
        caps.push(Implementations);
    }
    caps
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseOutput {
    pub ir: ParseResult,
    pub report: ParseReport,
}
impl Deref for ParseOutput {
    type Target = ParseResult;
    fn deref(&self) -> &ParseResult {
        &self.ir
    }
}
impl DerefMut for ParseOutput {
    fn deref_mut(&mut self) -> &mut ParseResult {
        &mut self.ir
    }
}
impl ParseOutput {
    pub(crate) fn new(path: &Path, source: &str, language: &str, grammar: &str) -> Self {
        Self {
            ir: ParseResult::default(),
            report: ParseReport::new(path, source, language, grammar),
        }
    }
    /// Explicit bridge for external adapters or cached legacy IR without reports.
    pub fn from_legacy(path: &Path, source: &str, language: &str, ir: ParseResult) -> Self {
        let mut report = ParseReport::new(path, source, language, "unknown-legacy-grammar");
        report.schema_version = 0;
        for disposition in report.capabilities.values_mut() {
            *disposition = CapabilityDisposition::Unknown;
        }
        let mut line = 1;
        let mut column = 1;
        for byte in source.bytes() {
            if byte == b'\n' {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        report.events.push(ParseEvent {
            code: ParseEventCode::UnknownLegacyReport,
            range: Location {
                file: path.to_path_buf(),
                line: 1,
                column: 1,
                end_line: line,
                end_column: column,
            },
            byte_start: 0,
            byte_end: source.len() as u64,
            affects: Capability::ALL.iter().copied().collect(),
            detail:
                "legacy IR has no capability/recovery provenance; reparse with a structured adapter"
                    .into(),
        });
        Self { ir, report }
    }
}

/// Walk the same original AST consumed by extraction, retaining actual recovery sites.
pub(crate) fn observe_tree(report: &mut ParseReport, root: tree_sitter::Node<'_>, source: &[u8]) {
    use Capability::*;
    use ParseEventCode::*;
    let closure = &[
        Calls,
        Imports,
        Exports,
        ValueReferences,
        Registration,
        ConsumerClosure,
        Resolution,
        Dispatch,
    ];
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.is_error() {
            report.event(
                node,
                ParserRecovery,
                Capability::ALL,
                "tree-sitter error recovery",
            );
        }
        if node.is_missing() {
            report.event(
                node,
                MissingNode,
                Capability::ALL,
                "tree-sitter inserted a missing token",
            );
        }
        let text =
            |n: tree_sitter::Node<'_>| std::str::from_utf8(&source[n.byte_range()]).unwrap_or("");
        match (report.language.as_str(), node.kind()) {
            (
                "typescript",
                "conditional_type" | "mapped_type_clause" | "infer_type" | "lookup_type"
                | "index_type_query",
            ) => report.event(
                node,
                UnsupportedSyntax,
                &[Types, Annotations, Implementations, Resolution],
                "advanced type evaluation is not modeled",
            ),
            ("typescript" | "javascript", "decorator") | ("python", "decorator") => {
                let mut affected = closure.to_vec();
                affected.extend([Types, Annotations]);
                report.event(
                    node,
                    UnsupportedDynamic,
                    &affected,
                    "decorator runtime registration/dispatch is not modeled",
                );
            }
            ("javascript" | "typescript", "import_statement") => {
                let mut cursor = node.walk();
                if !node
                    .named_children(&mut cursor)
                    .any(|n| n.kind() == "import_clause")
                {
                    report.event(
                        node,
                        UnsupportedSyntax,
                        &[Imports, DependencyEdges, ConsumerClosure],
                        "side-effect or import-equals dependency extraction is not modeled",
                    );
                }
            }
            ("javascript" | "typescript", "export_statement") => {
                if node
                    .child_by_field_name("value")
                    .is_some_and(|n| n.kind() != "identifier")
                {
                    report.event(
                        node,
                        UnsupportedSyntax,
                        &[Symbols, Exports, ConsumerClosure],
                        "default expression export identity is not modeled",
                    );
                }
            }
            ("javascript" | "typescript", "call_expression") => {
                if let Some(func) = node.child_by_field_name("function") {
                    let name = text(func);
                    if name == "import" || name == "require" {
                        let arg = node
                            .child_by_field_name("arguments")
                            .and_then(|a| a.named_child(0));
                        if name == "import" || arg.is_none_or(|a| a.kind() != "string") {
                            report.event(
                                node,
                                UnsupportedDynamic,
                                closure,
                                "dynamic module loading requires runtime resolution",
                            );
                        }
                    } else if func.kind() == "subscript_expression" {
                        report.event(
                            node,
                            UnsupportedDynamic,
                            closure,
                            "computed dispatch is not modeled",
                        );
                    }
                }
            }
            ("python", "call") => {
                if let Some(func) = node.child_by_field_name("function") {
                    if matches!(
                        text(func),
                        "importlib.import_module"
                            | "__import__"
                            | "getattr"
                            | "setattr"
                            | "exec"
                            | "eval"
                    ) {
                        report.event(
                            node,
                            UnsupportedDynamic,
                            closure,
                            "dynamic Python binding/module/dispatch requires runtime evidence",
                        );
                    }
                }
            }
            ("python", "assignment") => {
                if node
                    .child_by_field_name("left")
                    .is_some_and(|n| n.kind() == "attribute" || n.kind() == "subscript")
                {
                    report.event(
                        node,
                        UnsupportedDynamic,
                        closure,
                        "mutation of attribute/container may change consumer dispatch",
                    );
                }
            }
            ("rust", "macro_invocation" | "attribute_item" | "dynamic_type") => {
                let mut affected = closure.to_vec();
                affected.extend([Types, Implementations]);
                report.event(
                    node,
                    UnsupportedDynamic,
                    &affected,
                    "macro expansion or dynamic trait dispatch is not modeled",
                );
            }
            ("javascript" | "typescript", "assignment_expression") => {
                if node
                    .child_by_field_name("left")
                    .is_some_and(|n| n.kind() == "subscript_expression")
                {
                    report.event(
                        node,
                        UnsupportedDynamic,
                        closure,
                        "computed registration/export assignment is not modeled",
                    );
                }
            }
            _ => {}
        }
        let mut cursor = node.walk();
        pending.extend(node.children(&mut cursor));
    }
}
