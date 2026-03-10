// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Tree-sitter parsing and language adapters.
//!
//! Parsers produce language-agnostic IR from source files. The engine
//! never sees raw AST — only the IR produced here.

pub mod ir;
pub mod python;

use std::path::Path;

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
