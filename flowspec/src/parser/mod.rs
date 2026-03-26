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
