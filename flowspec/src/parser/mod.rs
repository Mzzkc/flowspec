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

/// Intermediate representation types — `Symbol`, `Scope`, `Reference`, `Boundary`, and their IDs.
pub mod ir;
/// JavaScript/TypeScript language adapter (ES modules, CommonJS, JSX/TSX).
pub mod javascript;
/// Python language adapter (imports, decorators, class hierarchy, attribute access).
pub mod python;
/// Rust language adapter (modules, traits, impls, use declarations).
pub mod rust;

use std::path::Path;

/// Maximum AST recursion depth for all language adapters.
///
/// When depth exceeds this limit during AST traversal, the adapter emits a
/// `tracing::warn!` and returns partial results for the current subtree.
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

/// Keep actual adapter traversal loss in the same IR that enters resolution.
/// Source coordinates come from the node that exceeded the real depth budget.
pub(crate) fn record_depth_loss(
    result: &mut ParseResult,
    path: &Path,
    node: tree_sitter::Node<'_>,
) {
    let start = node.start_position();
    let end = node.end_position();
    result.references.push(ir::Reference {
        id: ir::ReferenceId::default(),
        from: ir::SymbolId::default(),
        to: ir::SymbolId::default(),
        kind: ir::ReferenceKind::Read,
        location: ir::Location {
            file: path.to_path_buf(),
            line: start.row as u32 + 1,
            column: start.column as u32 + 1,
            end_line: end.row as u32 + 1,
            end_column: end.column as u32 + 1,
        },
        resolution: ir::ResolutionStatus::Partial(
            "prerequisite_loss:depth_truncation:parser_traversal".to_string(),
        ),
    });
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
}

fn registry_grammar_source(path: &Path, content: &str) -> Option<(tree_sitter::Language, String)> {
    let registry = crate::registry::Registry::default();
    let descriptor = registry.language_for_path(path)?;
    let language = match descriptor.grammar_version.as_str() {
        "tree-sitter-python/0.25.0" => tree_sitter_python::LANGUAGE.into(),
        "tree-sitter-rust/0.24.0" => tree_sitter_rust::LANGUAGE.into(),
        "tree-sitter-javascript/0.25.0" => tree_sitter_javascript::LANGUAGE.into(),
        _ => return None,
    };
    let source = if descriptor
        .limitations
        .iter()
        .any(|l| l == "typescript_preprocessing")
    {
        javascript::preprocess_typescript(content)
    } else {
        content.to_string()
    };
    Some((language, source))
}
