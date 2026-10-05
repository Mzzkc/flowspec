// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Tree-sitter parsing and language adapters — the first stage of the
//! Flowspec pipeline.
//!
//! Each language adapter translates tree-sitter AST into language-agnostic
//! IR ([`ir::ParseResult`]) containing symbols, scopes, references, and
//! boundaries. Downstream stages never see raw AST — only the IR produced
//! here. The [`ir`] module defines the shared types; each adapter module
//! (`python`, `javascript`, `rust`) implements [`LanguageAdapter`].
//!
//! **Pipeline position:** Parser → Graph → Analyzer → Manifest.
//! Output flows into [`crate::graph::populate_graph`] which inserts IR
//! nodes into the persistent analysis graph.

/// Source-owned normal-return control paths for explicit invocation policies.
pub mod control;
/// Intermediate representation types — `Symbol`, `Scope`, `Reference`, `Boundary`, and their IDs.
pub mod ir;
/// JavaScript/TypeScript language adapter (ES modules, CommonJS, JSX/TSX).
pub mod javascript;
/// Exact published duplication normal forms and located unresolved obligations.
pub mod normalization;
/// Python language adapter (imports, decorators, class hierarchy, attribute access).
pub mod python;
/// Structured per-file parser evidence.
pub mod report;
/// Rust language adapter (modules, traits, impls, use declarations).
pub mod rust;
/// Binding-aware semantic prerequisites, separate from compatibility entities.
pub mod semantic;
/// Native original-byte TypeScript and TSX extraction.
pub mod typescript;

use std::path::Path;

/// Maximum AST recursion depth for all language adapters.
///
/// When depth exceeds this limit during AST traversal, the adapter emits a
/// located `DepthTruncated` report event and returns partial IR for the subtree.
/// Sibling nodes continue processing normally. This prevents stack overflow
/// on adversarial input (e.g., 10,000-deep nested expressions) while
/// preserving useful output for the rest of the file.
pub const MAX_AST_DEPTH: usize = 256;

use crate::error::FlowspecError;
use ir::ParseResult;

/// Return true if any tree-sitter ERROR or missing recovery node exists.
pub(crate) fn has_parse_loss(root: tree_sitter::Node<'_>) -> bool {
    let mut pending = vec![root];
    while let Some(node) = pending.pop() {
        if node.is_error() || node.is_missing() {
            return true;
        }
        let mut cursor = node.walk();
        pending.extend(node.children(&mut cursor));
    }
    false
}

/// Check recovery in the grammar consumed by the current adapter.
/// This preserves the adapter IR API while report construction records loss.
pub(crate) fn source_has_parse_loss(path: &Path, content: &str) -> bool {
    let Some((language, source)) = registry_grammar_source(path, content) else {
        return true;
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return true;
    }
    match parser.parse(&source, None) {
        Some(tree) => has_parse_loss(tree.root_node()),
        None => true,
    }
}

/// First concrete tree-sitter recovery site in the bytes consumed by an adapter.
pub(crate) fn source_parse_loss_range(path: &Path, content: &str) -> Option<(u64, u64, u64, u64)> {
    let (language, source) = registry_grammar_source(path, content)?;
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language).ok()?;
    let tree = parser.parse(&source, None)?;
    let mut pending = vec![tree.root_node()];
    while let Some(node) = pending.pop() {
        if node.is_error() || node.is_missing() {
            let point = node.start_position();
            return Some((
                node.start_byte() as u64,
                node.end_byte() as u64,
                point.row as u64 + 1,
                point.column as u64 + 1,
            ));
        }
        let mut cursor = node.walk();
        pending.extend(node.children(&mut cursor));
    }
    None
}

/// Keep actual adapter traversal loss in the structured output consumed by analysis.
/// Source coordinates come from the node that exceeded the real depth budget.
pub(crate) fn record_depth_loss(
    result: &mut report::ParseOutput,
    path: &Path,
    node: tree_sitter::Node<'_>,
) {
    let _ = path;
    result.report.event(
        node,
        report::ParseEventCode::DepthTruncated,
        report::Capability::ALL,
        "actual adapter traversal exceeded MAX_AST_DEPTH",
    );
}

/// Trait implemented by each language adapter.
///
/// Language adapters translate tree-sitter AST into flowspec IR. This is one of
/// only two sanctioned traits in the codebase (the other is `OutputFormatter`),
/// per `conventions.yaml:27-31`.
///
/// Implementations must be `Send + Sync` to support concurrent file parsing
/// via `tokio::task::spawn_blocking`.
pub trait LanguageAdapter: Send + Sync {
    /// Returns the language name (e.g., `"python"`, `"javascript"`, `"rust"`).
    fn language_name(&self) -> &str;

    /// Returns `true` if this adapter can parse the file at the given path.
    fn can_handle(&self, path: &Path) -> bool;

    /// Parses a source file and returns IR nodes.
    ///
    /// The `content` parameter is the file's text. The adapter creates a
    /// tree-sitter parser internally (parsers are not `Send`).
    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError>;
    /// Structured analysis entry point. Legacy extensions retain IR compatibility
    /// while honestly exposing absent capability/recovery evidence.
    fn parse_output(
        &self,
        path: &Path,
        content: &str,
    ) -> Result<report::ParseOutput, FlowspecError> {
        Ok(report::ParseOutput::from_legacy(
            path,
            content,
            self.language_name(),
            self.parse_file(path, content)?,
        ))
    }
}

fn registry_grammar_source(path: &Path, content: &str) -> Option<(tree_sitter::Language, String)> {
    let registry = crate::registry::Registry::default();
    let descriptor = registry.language_for_path(path)?;
    let language = match descriptor.grammar_version.as_str() {
        "tree-sitter-python/0.25.0" => tree_sitter_python::LANGUAGE.into(),
        "tree-sitter-rust/0.24.0" => tree_sitter_rust::LANGUAGE.into(),
        "tree-sitter-javascript/0.25.0" => tree_sitter_javascript::LANGUAGE.into(),
        "tree-sitter-typescript/0.23.2+tsx" => typescript::grammar_for_path(path),
        _ => return None,
    };
    Some((language, content.to_string()))
}
