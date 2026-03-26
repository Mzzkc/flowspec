// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Python language adapter using tree-sitter-python.
//!
//! Extracts functions, classes, methods, imports, variables, constants, and
//! **function/method calls** from Python source files. Builds scope hierarchy.
//! Detects decorators. Uses Python naming conventions for visibility (leading
//! underscore = private) and constant detection (UPPER_CASE = constant).
//!
//! Call-site detection walks the full AST for `call` expression nodes and
//! emits `ReferenceKind::Call` references with callee names stored as
//! `ResolutionStatus::Partial("call:<name>")`. Intra-file resolution of
//! these references happens downstream in `populate_graph`.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::ir::*;
use super::LanguageAdapter;
use crate::error::FlowspecError;

/// Python language adapter backed by tree-sitter-python.
///
/// Implements the [`LanguageAdapter`] trait. Creates a fresh tree-sitter
/// parser per `parse_file` call (parsers are not `Send`).
pub struct PythonAdapter;

impl PythonAdapter {
    /// Creates a new Python adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for PythonAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language_name(&self) -> &str {
        "python"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension().map(|e| e == "py").unwrap_or(false)
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .map_err(|e| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: format!("failed to set Python language: {}", e),
            })?;

        let tree = parser
            .parse(content, None)
            .ok_or_else(|| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: "tree-sitter failed to produce a parse tree".to_string(),
            })?;

        let content_bytes = content.as_bytes();
        let file_stem = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "unknown".to_string());

        let mut result = ParseResult::default();

        // Create file-level scope
        let root = tree.root_node();
        result.scopes.push(Scope {
            id: ScopeId::default(),
            kind: ScopeKind::File,
            parent: None,
            name: path.to_string_lossy().to_string(),
            location: node_location(path, root),
        });

        // Walk the AST
        let mut scope_stack: Vec<usize> = vec![0]; // file scope index
        visit_children_top(
            &mut result,
            &mut scope_stack,
            content_bytes,
            path,
            &file_stem,
            root,
        );

        // Extract all function/method calls from the AST
        extract_all_calls(&mut result, content_bytes, path, root, 0);

        // Extract attribute accesses to track import usage (e.g., os.path.join, sys.argv)
        extract_attribute_accesses(&mut result, content_bytes, path, root, 0);

        // Post-processing: annotate imports inside TYPE_CHECKING blocks
        mark_type_checking_imports(&mut result, content_bytes, path, root);

        Ok(result)
    }
}

/// Extracts UTF-8 text from a tree-sitter node.
fn node_text<'a>(content: &'a [u8], node: Node) -> &'a str {
    let start = node.start_byte();
    let end = node.end_byte().min(content.len());
    std::str::from_utf8(&content[start..end]).unwrap_or("")
}

/// Converts a tree-sitter node to a 1-based Location.
fn node_location(path: &Path, node: Node) -> Location {
    let start = node.start_position();
    let end = node.end_position();
    Location {
        file: path.to_path_buf(),
        line: start.row as u32 + 1,
        column: start.column as u32 + 1,
        end_line: end.row as u32 + 1,
        end_column: end.column as u32 + 1,
    }
}

/// Determines visibility from a Python name.
fn python_visibility(name: &str) -> Visibility {
    if name.starts_with('_') {
        Visibility::Private
    } else {
        Visibility::Public
    }
}

/// Returns true if a name follows UPPER_CASE convention (constant).
fn is_python_constant(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_uppercase() || c == '_' || c.is_ascii_digit())
        && name.chars().any(|c| c.is_alphabetic())
}

/// Builds a qualified name from scope stack.
fn build_qualified_name(
    result: &ParseResult,
    scope_stack: &[usize],
    file_stem: &str,
    name: &str,
) -> String {
    let mut parts = vec![file_stem.to_string()];
    for &idx in scope_stack.iter().skip(1) {
        if idx < result.scopes.len() {
            parts.push(result.scopes[idx].name.clone());
        }
    }
    parts.push(name.to_string());
    parts.join("::")
}

/// Visit top-level children of a node.
fn visit_children_top(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_node(result, scope_stack, content, path, file_stem, child, &[], 0);
    }
}

/// Visit a single AST node and extract IR.
///
/// The `depth` parameter tracks recursion depth. When it exceeds
/// [`super::MAX_AST_DEPTH`], emits a warning and returns without
/// recursing further, preserving partial results for shallower nodes.
#[allow(clippy::too_many_arguments)]
fn visit_node(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    decorators: &[String],
    depth: usize,
) {
    if depth > super::MAX_AST_DEPTH {
        tracing::warn!(
            "AST depth limit ({}) reached at {}:{}",
            super::MAX_AST_DEPTH,
            path.display(),
            node.start_position().row + 1,
        );
        return;
    }

    match node.kind() {
        "function_definition" => {
            extract_function(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                decorators,
                depth,
            );
        }
        "class_definition" => {
            extract_class(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                decorators,
                depth,
            );
        }
        "import_statement" => {
            extract_import(result, content, path, node);
        }
        "import_from_statement" => {
            extract_import_from(result, content, path, node);
        }
        "decorated_definition" => {
            extract_decorated(result, scope_stack, content, path, file_stem, node, depth);
        }
        "expression_statement" => {
            // Check for assignment as direct child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if scope_stack.len() <= 1 {
                    match child.kind() {
                        "assignment" => {
                            // Check if this is __all__ = [...] before standard extraction
                            let is_dunder_all = child
                                .child_by_field_name("left")
                                .map(|left| {
                                    left.kind() == "identifier"
                                        && node_text(content, left) == "__all__"
                                })
                                .unwrap_or(false);
                            if is_dunder_all {
                                extract_dunder_all(result, content, path, child);
                            }
                            extract_assignment(
                                result,
                                scope_stack,
                                content,
                                path,
                                file_stem,
                                child,
                            );
                        }
                        "augmented_assignment" => {
                            // Check for __all__ += [...]
                            let is_dunder_all = child
                                .child_by_field_name("left")
                                .map(|left| {
                                    left.kind() == "identifier"
                                        && node_text(content, left) == "__all__"
                                })
                                .unwrap_or(false);
                            if is_dunder_all {
                                extract_dunder_all(result, content, path, child);
                            }
                        }
                        _ => {}
                    }
                } else if child.kind() == "assignment" {
                    // Non-module-level assignments (handled by extract_assignment's guard)
                }
            }
        }
        _ => {
            // Recurse into other nodes
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit_node(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    &[],
                    depth + 1,
                );
            }
        }
    }
}

/// Extract a function/method definition.
#[allow(clippy::too_many_arguments)]
fn extract_function(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    decorators: &[String],
    depth: usize,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(content, n).to_string())
        .unwrap_or_default();

    let params = node
        .child_by_field_name("parameters")
        .map(|n| node_text(content, n).to_string());

    let return_type = node
        .child_by_field_name("return_type")
        .map(|n| node_text(content, n).to_string());

    let signature = match (&params, &return_type) {
        (Some(p), Some(r)) => Some(format!("{} -> {}", p, r)),
        (Some(p), None) => Some(p.clone()),
        _ => None,
    };

    // Determine if method (parent scope is a class/Module scope)
    let is_method = scope_stack.len() > 1 && {
        let parent_idx = *scope_stack.last().unwrap_or(&0);
        parent_idx < result.scopes.len() && result.scopes[parent_idx].kind == ScopeKind::Module
    };

    let kind = if is_method {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    };

    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind,
        name: name.clone(),
        qualified_name,
        visibility: python_visibility(&name),
        signature,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: decorators.to_vec(),
    });

    // Create function scope for nested definitions
    let func_scope_idx = result.scopes.len();
    result.scopes.push(Scope {
        id: ScopeId::default(),
        kind: ScopeKind::Function,
        parent: None,
        name: name.clone(),
        location: node_location(path, node),
    });

    scope_stack.push(func_scope_idx);

    // Visit body
    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            visit_node(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                child,
                &[],
                depth + 1,
            );
        }
    }

    scope_stack.pop();
}

/// Extract a class definition.
#[allow(clippy::too_many_arguments)]
fn extract_class(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    decorators: &[String],
    depth: usize,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(content, n).to_string())
        .unwrap_or_default();

    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Class,
        name: name.clone(),
        qualified_name,
        visibility: python_visibility(&name),
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: decorators.to_vec(),
    });

    // Class scope (Module kind — classes define a namespace)
    let class_scope_idx = result.scopes.len();
    result.scopes.push(Scope {
        id: ScopeId::default(),
        kind: ScopeKind::Module,
        parent: None,
        name: name.clone(),
        location: node_location(path, node),
    });

    scope_stack.push(class_scope_idx);

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for child in body.children(&mut cursor) {
            visit_node(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                child,
                &[],
                depth + 1,
            );
        }
    }

    scope_stack.pop();
}

/// Extract `import module` statements.
///
/// Creates both a `Reference` (for graph edges) and a `Symbol` (for pattern detection)
/// for each imported name. The symbol uses `SymbolKind::Module` with an `"import"`
/// annotation so that `phantom_dependency` can find and check import symbols.
///
/// For aliased imports (`import os as o`), the symbol name is the alias (what code
/// references), not the original module name. The original module name is stored
/// as a `"from:<module>"` annotation for cross-file resolution. Aliased imports
/// additionally store `"original_name:<name>"` so the resolution pass can look up
/// the original name in the target module.
fn extract_import(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
            let (import_name, symbol_name, is_aliased) = if child.kind() == "aliased_import" {
                let original = child
                    .child_by_field_name("name")
                    .map(|n| node_text(content, n).to_string())
                    .unwrap_or_default();
                let alias = child
                    .child_by_field_name("alias")
                    .map(|n| node_text(content, n).to_string());
                let has_alias = alias.is_some();
                // Symbol name is the alias (what code uses), falling back to original
                let sym_name = alias.unwrap_or_else(|| original.clone());
                (original, sym_name, has_alias)
            } else {
                let name = node_text(content, child).to_string();
                (name.clone(), name, false)
            };

            if !import_name.is_empty() {
                result.references.push(Reference {
                    id: ReferenceId::default(),
                    from: SymbolId::default(),
                    to: SymbolId::default(),
                    kind: ReferenceKind::Import,
                    location: node_location(path, node),
                    resolution: ResolutionStatus::Partial("external".to_string()),
                });

                // Build annotations: "import" + "from:<module>" + optional "original_name:<name>"
                let mut annotations = vec!["import".to_string(), format!("from:{}", import_name)];
                if is_aliased {
                    annotations.push(format!("original_name:{}", import_name));
                }

                // Create import symbol for phantom_dependency detection
                result.symbols.push(Symbol {
                    id: SymbolId::default(),
                    kind: SymbolKind::Module,
                    name: symbol_name.clone(),
                    qualified_name: format!("{}::import::{}", file_stem, symbol_name),
                    visibility: Visibility::Private,
                    signature: None,
                    location: node_location(path, node),
                    resolution: ResolutionStatus::Partial("import".to_string()),
                    scope: ScopeId::default(),
                    annotations,
                });
            }
        }
    }
}

/// Extract `from module import name` statements.
///
/// Creates both a `Reference` and a `Symbol` for each imported name. For aliased
/// imports (`from pathlib import Path as P`), the symbol name is the alias.
/// Star imports (`from os import *`) produce only a Reference, no Symbol (no
/// specific name to track).
///
/// Each import symbol carries a `"from:<module>"` annotation recording the source
/// module name for cross-file resolution. Aliased imports additionally store
/// `"original_name:<name>"` so the resolution pass can look up the original
/// name in the target module.
fn extract_import_from(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let full_text = node_text(content, node);
    let is_relative = full_text.contains("from .") || full_text.starts_with("from .");
    let is_star = full_text.contains("import *");

    // Extract the source module name (e.g., "utils" from "from utils import helper")
    let module_name = node
        .child_by_field_name("module_name")
        .map(|m| node_text(content, m).to_string())
        .unwrap_or_default();

    if is_star {
        let mut annotations = vec!["import".to_string()];
        if !module_name.is_empty() {
            annotations.push(format!("from:{}", module_name));
        }
        result.references.push(Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Import,
            location: node_location(path, node),
            resolution: ResolutionStatus::Partial("star import".to_string()),
        });
        // Create a symbol for star imports so cross-file resolution can find the module
        if !module_name.is_empty() {
            result.symbols.push(Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Module,
                name: format!("*:{}", module_name),
                qualified_name: format!("{}::import::*:{}", file_stem, module_name),
                visibility: Visibility::Private,
                signature: None,
                location: node_location(path, node),
                resolution: ResolutionStatus::Partial("star import".to_string()),
                scope: ScopeId::default(),
                annotations,
            });
        }
        return;
    }

    // Find the module name node to determine where imported names start
    let module_end = node
        .child_by_field_name("module_name")
        .map(|m| m.end_byte())
        .unwrap_or(0);

    let mut cursor = node.walk();
    let mut found_names = false;
    for child in node.children(&mut cursor) {
        // Imported names appear after the module name
        if child.start_byte() <= module_end {
            continue;
        }

        match child.kind() {
            "dotted_name" | "identifier" => {
                let name = node_text(content, child).to_string();
                if !name.is_empty() && name != "import" {
                    let resolution = if is_relative {
                        ResolutionStatus::Partial("relative import".to_string())
                    } else {
                        ResolutionStatus::Partial("external".to_string())
                    };
                    result.references.push(Reference {
                        id: ReferenceId::default(),
                        from: SymbolId::default(),
                        to: SymbolId::default(),
                        kind: ReferenceKind::Import,
                        location: node_location(path, node),
                        resolution,
                    });

                    // Build annotations with from:<module>
                    let mut annotations = vec!["import".to_string()];
                    if !module_name.is_empty() {
                        annotations.push(format!("from:{}", module_name));
                    }

                    // Create import symbol for phantom_dependency detection
                    result.symbols.push(Symbol {
                        id: SymbolId::default(),
                        kind: SymbolKind::Module,
                        name: name.clone(),
                        qualified_name: format!("{}::import::{}", file_stem, name),
                        visibility: Visibility::Private,
                        signature: None,
                        location: node_location(path, node),
                        resolution: ResolutionStatus::Partial("import".to_string()),
                        scope: ScopeId::default(),
                        annotations,
                    });

                    found_names = true;
                }
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let original_name = node_text(content, name_node).to_string();
                    if !original_name.is_empty() {
                        let resolution = if is_relative {
                            ResolutionStatus::Partial("relative import".to_string())
                        } else {
                            ResolutionStatus::Partial("external".to_string())
                        };
                        result.references.push(Reference {
                            id: ReferenceId::default(),
                            from: SymbolId::default(),
                            to: SymbolId::default(),
                            kind: ReferenceKind::Import,
                            location: node_location(path, node),
                            resolution,
                        });

                        // Use alias as symbol name if available
                        let alias = child
                            .child_by_field_name("alias")
                            .map(|n| node_text(content, n).to_string());
                        let has_alias = alias.is_some();
                        let symbol_name = alias.unwrap_or_else(|| original_name.clone());

                        // Build annotations with from:<module> and optional original_name:<name>
                        let mut annotations = vec!["import".to_string()];
                        if !module_name.is_empty() {
                            annotations.push(format!("from:{}", module_name));
                        }
                        if has_alias {
                            annotations.push(format!("original_name:{}", original_name));
                        }

                        result.symbols.push(Symbol {
                            id: SymbolId::default(),
                            kind: SymbolKind::Module,
                            name: symbol_name.clone(),
                            qualified_name: format!("{}::import::{}", file_stem, symbol_name),
                            visibility: Visibility::Private,
                            signature: None,
                            location: node_location(path, node),
                            resolution: ResolutionStatus::Partial("import".to_string()),
                            scope: ScopeId::default(),
                            annotations,
                        });

                        found_names = true;
                    }
                }
            }
            _ => {}
        }
    }

    // Fallback: if we didn't find specific names, add one reference for the import
    if !found_names {
        let resolution = if is_relative {
            ResolutionStatus::Partial("relative import".to_string())
        } else {
            ResolutionStatus::Partial("external".to_string())
        };
        result.references.push(Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Import,
            location: node_location(path, node),
            resolution,
        });
    }
}

/// Extract decorated definitions (collects decorators, visits inner def/class).
fn extract_decorated(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    depth: usize,
) {
    let mut decorators = Vec::new();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "decorator" {
            let text = node_text(content, child).trim().to_string();
            let deco_name = text.strip_prefix('@').unwrap_or(&text).trim().to_string();
            decorators.push(deco_name);
        }
    }

    let mut cursor2 = node.walk();
    for child in node.children(&mut cursor2) {
        match child.kind() {
            "function_definition" => {
                extract_function(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    &decorators,
                    depth,
                );
            }
            "class_definition" => {
                extract_class(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    &decorators,
                    depth,
                );
            }
            _ => {}
        }
    }
}

/// Extract module-level assignments as variables/constants.
fn extract_assignment(
    result: &mut ParseResult,
    scope_stack: &[usize],
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
) {
    if scope_stack.len() > 1 {
        return;
    }

    let left = match node.child_by_field_name("left") {
        Some(n) => n,
        None => return,
    };

    if left.kind() != "identifier" {
        return;
    }

    let name = node_text(content, left).to_string();
    if name.is_empty() {
        return;
    }

    let kind = if is_python_constant(&name) {
        SymbolKind::Constant
    } else {
        SymbolKind::Variable
    };

    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind,
        name,
        qualified_name,
        visibility: Visibility::Public,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    });
}

/// Extracts names from `__all__` assignments at module level.
///
/// Detects `__all__ = [...]`, `__all__ = (...)`, and `__all__ += [...]` forms.
/// For each string literal in the list/tuple, creates a `ReferenceKind::Read`
/// reference with `ResolutionStatus::Partial("attribute_access:<name>")`.
/// This piggybacks on the existing `attribute_access:` resolution in
/// `populate_graph`, creating incoming edges on import symbols so that
/// `phantom_dependency` sees them as used.
///
/// Only processes module-level assignments (scope_stack depth <= 1).
/// Class-level `__all__` attributes are ignored.
fn extract_dunder_all(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    // `node` is an `assignment` or `augmented_assignment` with left = `__all__`
    let right = match node.child_by_field_name("right") {
        Some(n) => n,
        None => return,
    };

    // Accept both list [...] and tuple (...) forms
    if right.kind() != "list" && right.kind() != "tuple" {
        return;
    }

    let mut cursor = right.walk();
    for child in right.children(&mut cursor) {
        if child.kind() != "string" {
            continue;
        }
        // Extract the unquoted value from string_content child
        let name = if let Some(content_node) = child.child_by_field_name("content") {
            // tree-sitter 0.25 exposes string_content via 'content' field
            node_text(content, content_node).to_string()
        } else {
            // Fallback: find string_content child by kind
            let mut sc = child.walk();
            let mut found = String::new();
            for grandchild in child.children(&mut sc) {
                if grandchild.kind() == "string_content" {
                    found = node_text(content, grandchild).to_string();
                    break;
                }
            }
            found
        };

        if name.is_empty() {
            continue;
        }

        result.references.push(Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Read,
            location: node_location(path, node),
            resolution: ResolutionStatus::Partial(format!("attribute_access:{}", name)),
        });
    }
}

/// Post-processing pass to annotate imports inside `if TYPE_CHECKING:` blocks.
///
/// Walks the root AST to find `if_statement` nodes where the condition is
/// `TYPE_CHECKING` (identifier) or `typing.TYPE_CHECKING` (attribute).
/// Records byte ranges of consequence blocks. Then annotates any import
/// symbols whose location falls within those ranges with `"type_checking_import"`.
///
/// Also creates `attribute_access:` usage references for:
/// - `TYPE_CHECKING` itself (so the `from typing import TYPE_CHECKING` isn't phantom)
/// - Each import inside the TYPE_CHECKING block (so they aren't flagged phantom)
fn mark_type_checking_imports(result: &mut ParseResult, content: &[u8], path: &Path, root: Node) {
    // Phase 1: Find all TYPE_CHECKING guard block byte ranges
    let tc_ranges = find_type_checking_ranges(root, content);

    if tc_ranges.is_empty() {
        return;
    }

    // Phase 2: Create usage reference for TYPE_CHECKING itself
    // This covers both `TYPE_CHECKING` (identifier) and `typing.TYPE_CHECKING` (attribute)
    result.references.push(Reference {
        id: ReferenceId::default(),
        from: SymbolId::default(),
        to: SymbolId::default(),
        kind: ReferenceKind::Read,
        location: node_location(path, root),
        resolution: ResolutionStatus::Partial("attribute_access:TYPE_CHECKING".to_string()),
    });

    // Phase 3: Annotate import symbols inside TYPE_CHECKING blocks
    // and create usage references for them
    for sym in result.symbols.iter_mut() {
        if !sym.annotations.contains(&"import".to_string()) {
            continue;
        }

        let sym_start = sym.location.line;
        let sym_end = sym.location.end_line;

        for &(range_start_line, range_end_line) in &tc_ranges {
            // Check if symbol's location is within the TYPE_CHECKING block
            if sym_start >= range_start_line && sym_end <= range_end_line {
                sym.annotations.push("type_checking_import".to_string());

                // No need to check other ranges
                break;
            }
        }
    }

    // Phase 4: Create attribute_access references for TYPE_CHECKING imports
    // so they appear "used" and phantom_dependency doesn't flag them
    let tc_import_names: Vec<String> = result
        .symbols
        .iter()
        .filter(|s| s.annotations.contains(&"type_checking_import".to_string()))
        .map(|s| s.name.clone())
        .collect();

    for name in tc_import_names {
        result.references.push(Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Read,
            location: node_location(path, root),
            resolution: ResolutionStatus::Partial(format!("attribute_access:{}", name)),
        });
    }
}

/// Finds byte ranges (as 1-based line ranges) of `if TYPE_CHECKING:` consequence blocks.
///
/// Detects both `if TYPE_CHECKING:` (identifier condition) and
/// `if typing.TYPE_CHECKING:` (attribute condition). Skips negated forms
/// like `if not TYPE_CHECKING:`.
fn find_type_checking_ranges(node: Node, content: &[u8]) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    collect_type_checking_ranges(node, content, &mut ranges);
    ranges
}

/// Recursive helper for `find_type_checking_ranges`.
fn collect_type_checking_ranges(node: Node, content: &[u8], ranges: &mut Vec<(u32, u32)>) {
    if node.kind() == "if_statement" {
        if let Some(condition) = node.child_by_field_name("condition") {
            let is_tc = match condition.kind() {
                "identifier" => node_text(content, condition) == "TYPE_CHECKING",
                "attribute" => {
                    // typing.TYPE_CHECKING
                    let text = node_text(content, condition);
                    text == "typing.TYPE_CHECKING"
                }
                _ => false,
            };

            if is_tc {
                if let Some(consequence) = node.child_by_field_name("consequence") {
                    let start_line = consequence.start_position().row as u32 + 1;
                    let end_line = consequence.end_position().row as u32 + 1;
                    ranges.push((start_line, end_line));
                }
                return; // Don't recurse into this if_statement's children
            }
        }
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        collect_type_checking_ranges(child, content, ranges);
    }
}

/// Recursively walks the AST to extract all function/method call references.
///
/// Creates a `Reference` with `kind: ReferenceKind::Call` for each `call` node found.
/// The callee name is stored in `resolution: ResolutionStatus::Partial("call:<name>")`
/// for later resolution by `populate_graph`. Both `from` and `to` are left as
/// `SymbolId::default()` — `populate_graph` resolves them via location containment
/// and name matching respectively.
///
/// The `depth` parameter prevents stack overflow on deeply nested expressions.
/// When depth exceeds [`super::MAX_AST_DEPTH`], the subtree is skipped with a warning.
fn extract_all_calls(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    node: Node,
    depth: usize,
) {
    if depth > super::MAX_AST_DEPTH {
        tracing::warn!(
            "AST depth limit ({}) reached in call extraction at {}:{}",
            super::MAX_AST_DEPTH,
            path.display(),
            node.start_position().row + 1,
        );
        return;
    }

    if node.kind() == "call" {
        if let Some(func_node) = node.child_by_field_name("function") {
            if let Some(name) = extract_callee_name(content, func_node) {
                result.references.push(Reference {
                    id: ReferenceId::default(),
                    from: SymbolId::default(),
                    to: SymbolId::default(),
                    kind: ReferenceKind::Call,
                    location: node_location(path, node),
                    resolution: ResolutionStatus::Partial(format!("call:{}", name)),
                });
            }
        }
    }

    // Recurse into all children to find nested calls
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_all_calls(result, content, path, child, depth + 1);
    }
}

/// Extracts the callee name from a call expression's `function` field.
///
/// Returns `Some(name)` for identifiers (`func`) and attribute accesses (`obj.method`).
/// Returns `None` for complex expressions that cannot be statically resolved
/// (subscript, lambda, conditional, etc.).
fn extract_callee_name(content: &[u8], func_node: Node) -> Option<String> {
    match func_node.kind() {
        "identifier" => {
            let name = node_text(content, func_node);
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
        "attribute" => {
            let object = func_node.child_by_field_name("object")?;
            let attr = func_node.child_by_field_name("attribute")?;
            let obj_name = node_text(content, object);
            let attr_name = node_text(content, attr);
            if attr_name.is_empty() {
                None
            } else {
                Some(format!("{}.{}", obj_name, attr_name))
            }
        }
        _ => None,
    }
}

/// Recursively walks the AST to extract attribute access references.
///
/// For each `attribute` node (e.g., `os.path.join`, `sys.argv`), extracts the
/// root identifier by walking the `object` field chain. If the root identifier
/// matches an import symbol name, creates a `ReferenceKind::Read` reference with
/// `ResolutionStatus::Partial("attribute_access:<root>")`. This enables
/// `phantom_dependency` to see that the import is actually used.
///
/// Only creates references for root identifiers that match import symbol names,
/// avoiding false matches on local variables.
///
/// The `depth` parameter prevents stack overflow on deeply nested attribute chains.
fn extract_attribute_accesses(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    node: Node,
    depth: usize,
) {
    if depth > super::MAX_AST_DEPTH {
        tracing::warn!(
            "AST depth limit ({}) reached in attribute extraction at {}:{}",
            super::MAX_AST_DEPTH,
            path.display(),
            node.start_position().row + 1,
        );
        return;
    }

    if node.kind() == "attribute" {
        if let Some(root_name) = extract_attribute_root_identifier(content, node) {
            // Only create a reference if the root matches a known import symbol
            let has_import = result
                .symbols
                .iter()
                .any(|s| s.name == root_name && s.annotations.contains(&"import".to_string()));

            if has_import {
                result.references.push(Reference {
                    id: ReferenceId::default(),
                    from: SymbolId::default(),
                    to: SymbolId::default(),
                    kind: ReferenceKind::Read,
                    location: node_location(path, node),
                    resolution: ResolutionStatus::Partial(format!(
                        "attribute_access:{}",
                        root_name
                    )),
                });
            }
        }
        // Don't recurse into this attribute's children — the root is already handled.
        // But we DO need to recurse into siblings, which the caller handles.
        return;
    }

    // Recurse into all children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_attribute_accesses(result, content, path, child, depth + 1);
    }
}

/// Extracts the root identifier from an attribute access chain.
///
/// Walks the `object` field recursively until reaching an `identifier` node.
/// For `os.path.join`, the chain is:
///   attribute(object: attribute(object: identifier("os"), attr: "path"), attr: "join")
/// This function returns `Some("os")`.
///
/// Returns `None` if the root is not a simple identifier (e.g., `func().attr`).
fn extract_attribute_root_identifier(content: &[u8], node: Node) -> Option<String> {
    let object = node.child_by_field_name("object")?;
    match object.kind() {
        "identifier" => {
            let name = node_text(content, object);
            if name.is_empty() {
                None
            } else {
                Some(name.to_string())
            }
        }
        "attribute" => extract_attribute_root_identifier(content, object),
        _ => None, // func().attr, subscript, etc. — not a simple identifier root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_dir() -> PathBuf {
        // CARGO_MANIFEST_DIR points to flowspec/ crate dir; fixtures are at workspace root
        let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        manifest.parent().unwrap().join("tests/fixtures/python")
    }

    fn parse_fixture(filename: &str) -> ParseResult {
        let path = fixture_dir().join(filename);
        let content = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read fixture {}: {}", filename, e));
        let adapter = PythonAdapter::new();
        adapter
            .parse_file(&path, &content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", filename, e))
    }

    #[test]
    fn test_python_adapter_can_handle_py_file() {
        let adapter = PythonAdapter::new();
        assert!(adapter.can_handle(Path::new("foo.py")));
        assert!(adapter.can_handle(Path::new("src/deep/module.py")));
        assert!(adapter.can_handle(Path::new("__init__.py")));
    }

    #[test]
    fn test_python_adapter_rejects_non_py_files() {
        let adapter = PythonAdapter::new();
        assert!(!adapter.can_handle(Path::new("foo.rs")));
        assert!(!adapter.can_handle(Path::new("foo.js")));
        assert!(!adapter.can_handle(Path::new("foo.py.bak")));
        assert!(!adapter.can_handle(Path::new("Makefile")));
    }

    #[test]
    fn test_python_adapter_language_name() {
        assert_eq!(PythonAdapter::new().language_name(), "python");
    }

    // -- basic_functions.py -------------------------------------------------

    #[test]
    fn test_basic_functions_symbol_count() {
        let result = parse_fixture("basic_functions.py");
        let fns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 3, "basic_functions.py has 3 functions");
    }

    #[test]
    fn test_basic_functions_names() {
        let result = parse_fixture("basic_functions.py");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"greet"));
        assert!(names.contains(&"add"));
        assert!(names.contains(&"_private_helper"));
    }

    #[test]
    fn test_basic_functions_private_visibility() {
        let result = parse_fixture("basic_functions.py");
        let helper = result
            .symbols
            .iter()
            .find(|s| s.name == "_private_helper")
            .unwrap();
        assert_eq!(helper.visibility, Visibility::Private);
    }

    #[test]
    fn test_basic_functions_public_visibility() {
        let result = parse_fixture("basic_functions.py");
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(greet.visibility, Visibility::Public);
    }

    #[test]
    fn test_basic_functions_location_accuracy() {
        let result = parse_fixture("basic_functions.py");
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert_eq!(greet.location.line, 1);
        // Location stores the path as passed to parse_file
        assert!(
            greet.location.file.ends_with("basic_functions.py"),
            "Location file must end with basic_functions.py, got: {:?}",
            greet.location.file
        );
    }

    // -- classes.py ---------------------------------------------------------

    #[test]
    fn test_classes_symbol_kinds() {
        let result = parse_fixture("classes.py");
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .collect();
        assert_eq!(classes.len(), 2);
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(methods.len() >= 4, "got {} methods", methods.len());
    }

    #[test]
    fn test_classes_method_scope() {
        let result = parse_fixture("classes.py");
        let dog_speak = result
            .symbols
            .iter()
            .find(|s| s.name == "speak" && s.qualified_name.contains("Dog"))
            .expect("Must find Dog.speak");
        assert!(dog_speak.qualified_name.contains("Dog"));
    }

    #[test]
    fn test_classes_staticmethod_annotation() {
        let result = parse_fixture("classes.py");
        let species = result
            .symbols
            .iter()
            .find(|s| s.name == "species")
            .expect("Must find species");
        assert!(
            species
                .annotations
                .iter()
                .any(|a| a.contains("staticmethod")),
            "species must have @staticmethod, got: {:?}",
            species.annotations
        );
    }

    // -- imports.py ---------------------------------------------------------

    #[test]
    fn test_imports_reference_count() {
        let result = parse_fixture("imports.py");
        let imports: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(
            imports.len() >= 5,
            "imports.py needs >= 5 import refs, got {}",
            imports.len()
        );
    }

    #[test]
    fn test_imports_star_import_partial_resolution() {
        let result = parse_fixture("imports.py");
        let star = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(reason) if reason.contains("star"))
        });
        assert!(star, "Star import must produce Partial with 'star' reason");
    }

    #[test]
    fn test_imports_relative_import() {
        let result = parse_fixture("imports.py");
        let has_rel = result.references.iter().any(|r| {
            r.kind == ReferenceKind::Import && !matches!(r.resolution, ResolutionStatus::Resolved)
        });
        assert!(has_rel);
    }

    // -- empty.py -----------------------------------------------------------

    #[test]
    fn test_empty_file_no_symbols() {
        let result = parse_fixture("empty.py");
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_empty_file_no_references() {
        let result = parse_fixture("empty.py");
        assert!(result.references.is_empty());
    }

    #[test]
    fn test_empty_file_has_file_scope() {
        let result = parse_fixture("empty.py");
        assert!(!result.scopes.is_empty());
        assert_eq!(result.scopes[0].kind, ScopeKind::File);
    }

    // -- only_comments.py ---------------------------------------------------

    #[test]
    fn test_comments_only_no_symbols() {
        let result = parse_fixture("only_comments.py");
        assert!(result.symbols.is_empty());
    }

    #[test]
    fn test_comments_only_has_file_scope() {
        let result = parse_fixture("only_comments.py");
        assert!(!result.scopes.is_empty());
    }

    // -- syntax_errors.py ---------------------------------------------------

    #[test]
    fn test_syntax_errors_partial_parse() {
        let result = parse_fixture("syntax_errors.py");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"valid_function"));
        assert!(names.contains(&"another_valid"));
    }

    #[test]
    fn test_syntax_errors_no_panic() {
        let adapter = PythonAdapter::new();
        let path = fixture_dir().join("syntax_errors.py");
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(adapter.parse_file(&path, &content).is_ok());
    }

    // -- deeply_nested.py ---------------------------------------------------

    #[test]
    fn test_deeply_nested_all_functions_found() {
        let result = parse_fixture("deeply_nested.py");
        let fns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(fns.len(), 10, "deeply_nested.py has 10 nested functions");
    }

    #[test]
    fn test_deeply_nested_scope_chain() {
        let result = parse_fixture("deeply_nested.py");
        assert!(
            result.scopes.len() >= 11,
            "Need >= 11 scopes, got {}",
            result.scopes.len()
        );
    }

    // -- unicode_identifiers.py ---------------------------------------------

    #[test]
    fn test_unicode_function_name() {
        let result = parse_fixture("unicode_identifiers.py");
        let cafe = result
            .symbols
            .iter()
            .find(|s| s.name == "café")
            .expect("Must find 'café'");
        assert_eq!(cafe.kind, SymbolKind::Function);
    }

    #[test]
    fn test_unicode_class_name() {
        let result = parse_fixture("unicode_identifiers.py");
        let nono = result
            .symbols
            .iter()
            .find(|s| s.name == "Ñoño")
            .expect("Must find 'Ñoño'");
        assert_eq!(nono.kind, SymbolKind::Class);
    }

    // -- constants_and_variables.py -----------------------------------------

    #[test]
    fn test_constants_detection() {
        let result = parse_fixture("constants_and_variables.py");
        let consts: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Constant)
            .collect();
        assert!(
            consts.len() >= 3,
            "Need >= 3 constants, got {}",
            consts.len()
        );
    }

    #[test]
    fn test_variables_detection() {
        let result = parse_fixture("constants_and_variables.py");
        let vars: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Variable)
            .collect();
        assert!(vars.len() >= 2, "Need >= 2 variables, got {}", vars.len());
    }

    // -- decorators.py ------------------------------------------------------

    #[test]
    fn test_multiple_decorators() {
        let result = parse_fixture("decorators.py");
        let decorated = result
            .symbols
            .iter()
            .find(|s| s.name == "decorated_function")
            .expect("Must find decorated_function");
        assert!(
            decorated.annotations.len() >= 2,
            "Need >= 2 annotations, got {}",
            decorated.annotations.len()
        );
    }

    #[test]
    fn test_property_decorator() {
        let result = parse_fixture("decorators.py");
        let value = result
            .symbols
            .iter()
            .find(|s| s.name == "value")
            .expect("Must find 'value'");
        assert!(value.annotations.iter().any(|a| a.contains("property")));
    }

    // -- Adversarial --------------------------------------------------------

    #[test]
    fn test_parse_binary_content_no_crash() {
        let adapter = PythonAdapter::new();
        let binary = String::from_utf8_lossy(b"\x00\x01\x02\x03\xff\xfe\xfd").to_string();
        let _result = adapter.parse_file(Path::new("binary.py"), &binary);
    }

    #[test]
    fn test_parse_extremely_long_line() {
        let adapter = PythonAdapter::new();
        let content = format!("x = \"{}\"", "a".repeat(100_000));
        assert!(adapter.parse_file(Path::new("long.py"), &content).is_ok());
    }

    #[test]
    fn test_parse_file_with_null_bytes() {
        let adapter = PythonAdapter::new();
        let _result = adapter.parse_file(
            Path::new("nulls.py"),
            "def foo():\n    pass\n\x00\ndef bar():\n    pass",
        );
    }

    // -- Qualified names ----------------------------------------------------

    #[test]
    fn test_qualified_name_format_top_level_function() {
        let result = parse_fixture("basic_functions.py");
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert!(greet.qualified_name.contains("basic_functions"));
        assert!(greet.qualified_name.contains("greet"));
    }

    #[test]
    fn test_qualified_name_format_method() {
        let result = parse_fixture("classes.py");
        let init = result
            .symbols
            .iter()
            .find(|s| s.name == "__init__" && s.qualified_name.contains("Animal"))
            .unwrap();
        assert!(init.qualified_name.contains("classes"));
        assert!(init.qualified_name.contains("Animal"));
        assert!(init.qualified_name.contains("__init__"));
    }

    // -- Helper for inline content tests ------------------------------------

    fn parse_fixture_content(filename: &str, content: &str) -> ParseResult {
        let adapter = PythonAdapter::new();
        let path = PathBuf::from(filename);
        adapter
            .parse_file(&path, content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", filename, e))
    }

    // -- Attribute access tracking (Cycle 5 D2) -----------------------------

    /// Single-level attribute call: `import json; json.dumps(data)`
    /// Adapter must produce a reference linking the attribute access to the `json` import.
    #[test]
    fn test_attribute_call_single_level_creates_reference() {
        let content = "import json\n\ndef serialize(data):\n    return json.dumps(data)\n";
        let result = parse_fixture_content("attr_call.py", content);

        let import_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "json" && s.annotations.contains(&"import".to_string()));
        assert!(import_sym.is_some(), "Must create import symbol for 'json'");

        let json_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(info) if info.contains("json"))
                    && matches!(r.kind, ReferenceKind::Read | ReferenceKind::Call)
            })
            .collect();

        assert!(
            !json_refs.is_empty(),
            "Must create at least one reference for 'json.dumps()' attribute access. \
             Got references: {:?}",
            result
                .references
                .iter()
                .map(|r| (&r.kind, &r.resolution))
                .collect::<Vec<_>>()
        );
    }

    /// Multi-level attribute chain: `import os; os.path.join("/tmp", "f")`
    /// Root identifier `os` must be tracked as used.
    #[test]
    fn test_attribute_chain_multi_level_creates_reference() {
        let content =
            "import os\n\ndef make_path():\n    return os.path.join(\"/tmp\", \"file\")\n";
        let result = parse_fixture_content("attr_chain.py", content);

        let os_import = result
            .symbols
            .iter()
            .find(|s| s.name == "os" && s.annotations.contains(&"import".to_string()));
        assert!(os_import.is_some(), "Must create import symbol for 'os'");

        let os_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(info) if info.contains("os"))
                    && matches!(r.kind, ReferenceKind::Read | ReferenceKind::Call)
            })
            .collect();

        assert!(
            !os_refs.is_empty(),
            "Must create reference for 'os.path.join()' — root identifier 'os' must be tracked"
        );
    }

    /// Bare attribute access (no call): `import sys; sys.argv`
    /// This is NOT inside a call node — `extract_all_calls()` alone won't catch it.
    #[test]
    fn test_bare_attribute_access_creates_reference() {
        let content = "import sys\n\ndef get_args():\n    return sys.argv[1:]\n";
        let result = parse_fixture_content("bare_attr.py", content);

        let sys_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(info) if info.contains("sys"))
                    && matches!(r.kind, ReferenceKind::Read | ReferenceKind::Call)
            })
            .collect();

        assert!(
            !sys_refs.is_empty(),
            "Bare attribute access 'sys.argv' must create a reference. \
             This is the critical case — sys.argv is NOT a call, so extract_all_calls() alone misses it."
        );
    }

    /// Aliased import: `import os as opsys; opsys.path.exists("/tmp")`
    /// The alias name is what appears in code; it must be tracked.
    #[test]
    fn test_aliased_import_attribute_access() {
        let content = "import os as opsys\n\ndef run():\n    return opsys.path.exists(\"/tmp\")\n";
        let result = parse_fixture_content("alias_attr.py", content);

        let alias_import = result
            .symbols
            .iter()
            .find(|s| s.name == "opsys" && s.annotations.contains(&"import".to_string()));
        assert!(
            alias_import.is_some(),
            "Import symbol name must be the alias 'opsys'"
        );

        let refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(info) if info.contains("opsys"))
            })
            .collect();

        assert!(
            !refs.is_empty(),
            "Attribute access on alias 'opsys.path.exists()' must create a reference"
        );
    }

    // -- Qualified name includes extension (Cycle 5 D4) ---------------------

    /// After fix: qualified name must use file_name() not file_stem().
    /// `app.py` → qualified name starts with "app.py::" not "app::"
    #[test]
    fn test_qualified_name_includes_extension() {
        let content = "def hello():\n    return \"hi\"\n";
        let path = PathBuf::from("app.py");
        let adapter = PythonAdapter::new();
        let result = adapter.parse_file(&path, content).unwrap();

        let hello = result
            .symbols
            .iter()
            .find(|s| s.name == "hello")
            .expect("Must find function 'hello'");

        assert!(
            hello.qualified_name.starts_with("app.py::"),
            "Qualified name must start with 'app.py::' (includes extension), got: {}",
            hello.qualified_name
        );
    }

    // =========================================================================
    // QA-1 Cycle 20: __all__ + TYPE_CHECKING tests
    // =========================================================================

    // -- Category 1: __all__ Basic Extraction (ALL-*) -------------------------

    #[test]
    fn test_dunder_all_basic_creates_export_references() {
        let result = parse_fixture("dunder_all_basic.py");
        let has_all_ref = result.references.iter().any(|r| {
            r.kind == ReferenceKind::Read
                && matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
        });
        assert!(
            has_all_ref,
            "Symbols listed in __all__ must create attribute_access references. \
             References: {:?}",
            result
                .references
                .iter()
                .filter(|r| r.kind == ReferenceKind::Read)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_dunder_all_non_listed_symbol_no_reference() {
        let result = parse_fixture("dunder_all_basic.py");
        let has_internal_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:internal_only"))
        });
        assert!(
            !has_internal_ref,
            "Symbols NOT in __all__ must NOT get attribute_access references from __all__ extraction"
        );
    }

    #[test]
    fn test_dunder_all_variable_extracted_as_symbol() {
        let result = parse_fixture("dunder_all_basic.py");
        let has_all_symbol = result.symbols.iter().any(|s| s.name == "__all__");
        assert!(
            has_all_symbol,
            "The __all__ variable itself must be extracted as a symbol"
        );
    }

    #[test]
    fn test_dunder_all_augmented_assignment() {
        let result = parse_fixture("dunder_all_augmented.py");
        let has_foo = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Foo"))
        });
        let has_bar = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Bar"))
        });
        assert!(has_foo, "Foo from __all__ = [...] must have a reference");
        assert!(has_bar, "Bar from __all__ += [...] must have a reference");
    }

    #[test]
    fn test_dunder_all_augmented_not_in_base_no_false_positive() {
        let result = parse_fixture("dunder_all_augmented.py");
        let has_baz = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Baz"))
        });
        assert!(
            !has_baz,
            "Baz is not in any __all__ list, must not get an attribute_access reference"
        );
    }

    // -- Category 2: __all__ Adversarial Edge Cases (AADV-*) ------------------

    #[test]
    fn test_dunder_all_empty_no_export_references() {
        let result = parse_fixture("dunder_all_empty.py");
        let has_helper_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
        });
        assert!(
            !has_helper_ref,
            "Empty __all__ must not create any export references for helper"
        );
    }

    #[test]
    fn test_dunder_all_non_string_items_skipped() {
        let result = parse_fixture("dunder_all_adversarial.py");
        let has_valid = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Valid"))
        });
        assert!(
            has_valid,
            "String 'Valid' must be extracted from __all__ despite non-string siblings"
        );
    }

    #[test]
    fn test_dunder_all_inside_class_ignored() {
        let result = parse_fixture("dunder_all_nested.py");
        let has_helper_export = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
        });
        assert!(
            !has_helper_export,
            "Class-level __all__ must not create module-level export references"
        );
    }

    #[test]
    fn test_dunder_all_tuple_form() {
        let result = parse_fixture("dunder_all_tuple.py");
        let has_foo = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Foo"))
        });
        let has_bar = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Bar"))
        });
        assert!(has_foo, "Foo from tuple __all__ must have a reference");
        assert!(has_bar, "Bar from tuple __all__ must have a reference");
    }

    #[test]
    fn test_dunder_all_duplicates_no_panic() {
        let result = parse_fixture("dunder_all_duplicates.py");
        let helper_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
            })
            .collect();
        assert!(
            !helper_refs.is_empty(),
            "Duplicate __all__ entries must still create at least one reference"
        );
    }

    #[test]
    fn test_dunder_all_single_quotes() {
        let source = "from models import Foo\n__all__ = ['Foo']\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test_single_quote.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let has_foo = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Foo"))
        });
        assert!(
            has_foo,
            "Single-quoted strings in __all__ must be extracted"
        );
    }

    #[test]
    fn test_dunder_all_multiple_assignments() {
        let source = "from a import X\nfrom b import Y\n__all__ = ['X']\n__all__ = ['Y']\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test_multi_all.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let has_y = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:Y"))
        });
        assert!(
            has_y,
            "At least the last __all__ assignment must produce references"
        );
    }

    // -- Category 3: TYPE_CHECKING Basic Detection (TC-*) ---------------------

    #[test]
    fn test_type_checking_imports_annotated() {
        let result = parse_fixture("type_checking_basic.py");
        let pathlike = result
            .symbols
            .iter()
            .find(|s| s.name == "PathLike" && s.annotations.contains(&"import".to_string()));
        assert!(pathlike.is_some(), "PathLike import must exist");
        let pathlike = pathlike.unwrap();
        assert!(
            pathlike
                .annotations
                .contains(&"type_checking_import".to_string()),
            "PathLike inside TYPE_CHECKING block must have 'type_checking_import' annotation. \
             Got annotations: {:?}",
            pathlike.annotations
        );
    }

    #[test]
    fn test_type_checking_import_has_usage_reference() {
        let result = parse_fixture("type_checking_basic.py");
        let has_pathlike_ref = result.references.iter().any(|r| {
            r.kind == ReferenceKind::Read
                && matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:PathLike"))
        });
        assert!(
            has_pathlike_ref,
            "TYPE_CHECKING imports must have attribute_access references to prevent phantom flagging"
        );
    }

    #[test]
    fn test_regular_imports_no_type_checking_annotation() {
        let result = parse_fixture("type_checking_basic.py");
        let os_import = result
            .symbols
            .iter()
            .find(|s| s.name == "os" && s.annotations.contains(&"import".to_string()));
        assert!(os_import.is_some(), "os import must exist");
        let os_import = os_import.unwrap();
        assert!(
            !os_import
                .annotations
                .contains(&"type_checking_import".to_string()),
            "Regular imports outside TYPE_CHECKING must NOT get type_checking_import annotation"
        );
    }

    #[test]
    fn test_type_checking_name_has_usage_reference() {
        let result = parse_fixture("type_checking_basic.py");
        let has_tc_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:TYPE_CHECKING"))
        });
        assert!(
            has_tc_ref,
            "TYPE_CHECKING itself must have a usage reference (the if-guard uses it)"
        );
    }

    #[test]
    fn test_type_checking_import_symbol_exists() {
        let result = parse_fixture("type_checking_basic.py");
        let tc_sym = result.symbols.iter().find(|s| s.name == "TYPE_CHECKING");
        assert!(tc_sym.is_some(), "TYPE_CHECKING symbol must be extracted");
        let tc_sym = tc_sym.unwrap();
        assert!(
            tc_sym.annotations.contains(&"import".to_string()),
            "TYPE_CHECKING must have 'import' annotation"
        );
        assert!(
            tc_sym.annotations.iter().any(|a| a.contains("from:typing")),
            "TYPE_CHECKING must have 'from:typing' annotation"
        );
    }

    // -- Category 4: TYPE_CHECKING Adversarial Edge Cases (TCADV-*) -----------

    #[test]
    fn test_type_checking_no_imports_no_crash() {
        let result = parse_fixture("type_checking_no_imports.py");
        let tc_annotated: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"type_checking_import".to_string()))
            .collect();
        assert!(
            tc_annotated.is_empty(),
            "No imports in TYPE_CHECKING block → no type_checking_import annotations. Got: {:?}",
            tc_annotated.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_type_checking_attribute_form() {
        let result = parse_fixture("type_checking_attribute_form.py");
        let request = result
            .symbols
            .iter()
            .find(|s| s.name == "Request" && s.annotations.contains(&"import".to_string()));
        assert!(request.is_some(), "Request import must exist");
        let request = request.unwrap();
        assert!(
            request
                .annotations
                .contains(&"type_checking_import".to_string()),
            "Imports inside `typing.TYPE_CHECKING` block must have type_checking_import annotation. \
             Got: {:?}",
            request.annotations
        );
    }

    #[test]
    fn test_type_checking_negated_not_a_guard() {
        let result = parse_fixture("type_checking_negated.py");
        let getcwd = result
            .symbols
            .iter()
            .find(|s| s.name == "getcwd" && s.annotations.contains(&"import".to_string()));
        assert!(getcwd.is_some(), "getcwd import must exist");
        let getcwd = getcwd.unwrap();
        assert!(
            !getcwd
                .annotations
                .contains(&"type_checking_import".to_string()),
            "Imports in `if not TYPE_CHECKING:` are runtime imports, NOT type-only"
        );
    }

    #[test]
    fn test_type_checking_user_assignment_no_crash() {
        let result = parse_fixture("type_checking_assignment.py");
        let has_tc_var = result.symbols.iter().any(|s| s.name == "TYPE_CHECKING");
        assert!(
            has_tc_var,
            "TYPE_CHECKING variable assignment should be extracted"
        );
    }

    #[test]
    fn test_type_checking_plain_import_statement() {
        let source = "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    import sys\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test_tc_plain_import.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let sys_sym = result
            .symbols
            .iter()
            .find(|s| s.name == "sys" && s.annotations.contains(&"import".to_string()));
        assert!(sys_sym.is_some(), "sys import must exist");
        let sys_sym = sys_sym.unwrap();
        assert!(
            sys_sym
                .annotations
                .contains(&"type_checking_import".to_string()),
            "Plain import inside TYPE_CHECKING must also get type_checking_import annotation"
        );
    }

    #[test]
    fn test_type_checking_nested_if_still_in_scope() {
        let source = concat!(
            "from typing import TYPE_CHECKING\nimport sys\n\n",
            "if TYPE_CHECKING:\n",
            "    if sys.version_info >= (3, 9):\n",
            "        from collections.abc import Sequence\n",
        );
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("test_tc_nested.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let seq = result
            .symbols
            .iter()
            .find(|s| s.name == "Sequence" && s.annotations.contains(&"import".to_string()));
        assert!(seq.is_some(), "Sequence import must exist");
        let seq = seq.unwrap();
        assert!(
            seq.annotations
                .contains(&"type_checking_import".to_string()),
            "Nested import inside TYPE_CHECKING block must still be annotated"
        );
    }

    // -- Category 5: Integration Tests (INT-*) --------------------------------

    #[test]
    fn test_combined_all_and_type_checking() {
        let result = parse_fixture("combined_all_typechecking.py");

        // __all__ creates reference for helper
        let has_helper_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
        });
        assert!(
            has_helper_ref,
            "helper in __all__ must have attribute_access reference"
        );

        // PathLike inside TYPE_CHECKING gets annotation
        let pathlike = result
            .symbols
            .iter()
            .find(|s| s.name == "PathLike" && s.annotations.contains(&"import".to_string()));
        assert!(pathlike.is_some(), "PathLike import must exist");
        assert!(
            pathlike
                .unwrap()
                .annotations
                .contains(&"type_checking_import".to_string()),
            "PathLike must have type_checking_import annotation"
        );

        // TYPE_CHECKING itself has usage reference
        let has_tc_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:TYPE_CHECKING"))
        });
        assert!(has_tc_ref, "TYPE_CHECKING must have usage reference");

        // unused_thing has no __all__ or TYPE_CHECKING protection
        let unused_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| {
                matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:unused_thing"))
            })
            .collect();
        assert!(
            unused_refs.is_empty(),
            "unused_thing must NOT have any attribute_access references from __all__ or TYPE_CHECKING"
        );
    }

    #[test]
    fn test_reexport_init_regression() {
        let result = parse_fixture("reexport_init.py");
        let has_all = result.symbols.iter().any(|s| s.name == "__all__");
        assert!(
            has_all,
            "reexport_init.py must still extract __all__ symbol"
        );

        let has_helper_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:helper"))
        });
        assert!(
            has_helper_ref,
            "reexport_init.py: helper in __all__ must get attribute_access reference after implementation"
        );
    }

    #[test]
    fn test_unused_import_fixture_unaffected() {
        let result = parse_fixture("unused_import.py");
        let import_syms: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"import".to_string()))
            .collect();
        assert_eq!(
            import_syms.len(),
            5,
            "unused_import.py must still have exactly 5 import symbols"
        );
        let tc_annotated: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"type_checking_import".to_string()))
            .collect();
        assert!(
            tc_annotated.is_empty(),
            "No TYPE_CHECKING in file → no type_checking_import annotations"
        );
    }

    // -- Category 6: Inline Source Unit Tests (INLINE-*) ----------------------

    #[test]
    fn test_inline_minimal_dunder_all() {
        let source = "from x import A\n__all__ = [\"A\"]\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("inline_min_all.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let has_a_ref = result.references.iter().any(|r| {
            matches!(&r.resolution, ResolutionStatus::Partial(s) if s.contains("attribute_access:A"))
        });
        assert!(has_a_ref, "Minimal __all__ must create reference for A");
    }

    #[test]
    fn test_inline_minimal_type_checking() {
        let source =
            "from typing import TYPE_CHECKING\n\nif TYPE_CHECKING:\n    from os import PathLike\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("inline_min_tc.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let pathlike = result
            .symbols
            .iter()
            .find(|s| s.name == "PathLike" && s.annotations.contains(&"import".to_string()))
            .expect("PathLike import must exist");
        assert!(
            pathlike
                .annotations
                .contains(&"type_checking_import".to_string()),
            "Minimal TYPE_CHECKING must annotate import"
        );
    }

    #[test]
    fn test_inline_dunder_all_annotated_assignment() {
        let source = "from x import Foo\n__all__: list[str] = [\"Foo\"]\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("inline_annotated_all.py");
        let result = adapter.parse_file(&path, source);
        assert!(
            result.is_ok(),
            "Annotated __all__ assignment must not crash"
        );
    }

    #[test]
    fn test_inline_dunder_all_no_imports() {
        let source = "__all__ = [\"Foo\", \"Bar\"]\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("inline_all_no_imports.py");
        let result = adapter.parse_file(&path, source).unwrap();
        assert!(result.symbols.iter().any(|s| s.name == "__all__"));
    }

    #[test]
    fn test_inline_type_checking_with_else() {
        let source = concat!(
            "from typing import TYPE_CHECKING\n\n",
            "if TYPE_CHECKING:\n",
            "    from os import PathLike\n",
            "else:\n",
            "    PathLike = object\n",
        );
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("inline_tc_else.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let pathlike_import = result
            .symbols
            .iter()
            .find(|s| s.name == "PathLike" && s.annotations.contains(&"import".to_string()));
        if let Some(sym) = pathlike_import {
            assert!(
                sym.annotations
                    .contains(&"type_checking_import".to_string()),
                "Import in if-branch of TYPE_CHECKING must be annotated even with else"
            );
        }
    }

    // -- Category 7: Regression Guards (REG-*) --------------------------------

    #[test]
    fn test_dunder_all_variable_kind_correct() {
        let source = "from x import A\n__all__ = [\"A\"]\n";
        let adapter = PythonAdapter::new();
        let path = PathBuf::from("reg_all_kind.py");
        let result = adapter.parse_file(&path, source).unwrap();
        let all_sym = result.symbols.iter().find(|s| s.name == "__all__").unwrap();
        assert_eq!(
            all_sym.kind,
            SymbolKind::Variable,
            "__all__ must remain SymbolKind::Variable (not Constant)"
        );
    }

    #[test]
    fn test_regression_import_count_basic_functions() {
        let result = parse_fixture("basic_functions.py");
        let import_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert_eq!(
            import_refs.len(),
            0,
            "basic_functions.py has no imports — count must remain 0"
        );
    }

    #[test]
    fn test_regression_empty_file_with_new_features() {
        let result = parse_fixture("empty.py");
        assert!(
            result.symbols.is_empty(),
            "Empty file must still produce no symbols"
        );
        assert!(
            result.references.is_empty(),
            "Empty file must still produce no references"
        );
    }

    #[test]
    fn test_regression_syntax_errors_with_new_features() {
        let result = parse_fixture("syntax_errors.py");
        let names: Vec<_> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"valid_function"),
            "valid_function must survive syntax errors"
        );
        assert!(
            names.contains(&"another_valid"),
            "another_valid must survive syntax errors"
        );
    }
}
