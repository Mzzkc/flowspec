// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Rust language adapter using tree-sitter-rust.
//!
//! Extracts functions, structs, enums, traits, impl methods, constants,
//! statics, and use statements from Rust source files. Builds scope
//! hierarchy. Detects visibility modifiers (`pub` = Public, `pub(crate)` = Crate,
//! `pub(super)` / `pub(in path)` = Protected, no modifier = Private).
//!
//! Impl block association: functions inside `impl Foo { ... }` are extracted
//! as `SymbolKind::Method` with qualified names containing the struct name
//! (e.g., `module.rs::Foo::method`).
//!
//! Call-site detection walks the full AST for `call_expression` nodes and
//! emits `ReferenceKind::Call` references with callee names stored as
//! `ResolutionStatus::Partial("call:<name>")`. Intra-file resolution happens
//! downstream in `populate_graph`.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::ir::*;
use super::LanguageAdapter;
use crate::error::FlowspecError;

/// Rust language adapter backed by tree-sitter-rust.
///
/// Implements the [`LanguageAdapter`] trait. Creates a fresh tree-sitter
/// parser per `parse_file` call (parsers are not `Send`). Accepts `.rs`
/// file extension (case-sensitive, lowercase only).
pub struct RustAdapter;

impl RustAdapter {
    /// Creates a new Rust adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for RustAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for RustAdapter {
    fn language_name(&self) -> &str {
        "rust"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "rs")
            .unwrap_or(false)
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .map_err(|e| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: format!("failed to set Rust language: {}", e),
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
        visit_children(
            &mut result,
            &mut scope_stack,
            content_bytes,
            path,
            &file_stem,
            root,
            None, // not inside impl block
            0,
        );

        // Extract all function/method calls from the AST
        extract_all_calls(&mut result, content_bytes, path, root, 0);

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

/// Extract visibility from a node's `visibility_modifier` child.
///
/// - `pub` → `Visibility::Public`
/// - `pub(crate)` → `Visibility::Crate`
/// - `pub(super)` or `pub(in path)` → `Visibility::Protected`
/// - No modifier → `Visibility::Private`
fn extract_visibility(content: &[u8], node: Node) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(content, child).trim();
            if text == "pub" {
                return Visibility::Public;
            } else if text == "pub(crate)" {
                return Visibility::Crate;
            } else {
                // pub(super), pub(in path) → Protected
                return Visibility::Protected;
            }
        }
    }
    Visibility::Private
}

/// Extract the name identifier from a node, stripping generic parameters.
///
/// For `struct Foo<T>`, returns "Foo" not "Foo<T>".
fn extract_name(content: &[u8], node: Node) -> Option<String> {
    // Try the "name" field first (used by function_item, struct_item, etc.)
    if let Some(name_node) = node.child_by_field_name("name") {
        let text = node_text(content, name_node).to_string();
        return Some(text);
    }
    None
}

/// Extract the impl type name, stripping generic parameters.
///
/// For `impl<T> Container<T>`, returns "Container".
/// For `impl Display for Point`, returns "Point" (the implementing type).
fn extract_impl_type_name(content: &[u8], node: Node) -> Option<String> {
    // For `impl Trait for Type`, the type is in the "type" field
    // For `impl Type`, the type is also in the "type" field
    if let Some(type_node) = node.child_by_field_name("type") {
        let text = node_text(content, type_node);
        // Strip generic params: "Container<T>" → "Container"
        let name = text.split('<').next().unwrap_or(text).trim();
        // Handle scoped types: "std::fmt::Display" → take just "Display" for qualified names
        // but for impl blocks, we want the full type name for the implementing type
        return Some(name.to_string());
    }
    None
}

/// Extract function signature string from a function_item node.
fn extract_signature(content: &[u8], node: Node) -> Option<String> {
    let params = node.child_by_field_name("parameters");
    let ret = node.child_by_field_name("return_type");

    match (params, ret) {
        (Some(p), Some(r)) => Some(format!(
            "{} -> {}",
            node_text(content, p),
            node_text(content, r)
        )),
        (Some(p), None) => Some(node_text(content, p).to_string()),
        (None, Some(r)) => Some(format!("() -> {}", node_text(content, r))),
        (None, None) => None,
    }
}

/// Visit all children of a node and extract IR.
#[allow(clippy::too_many_arguments)]
fn visit_children(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    impl_type: Option<&str>,
    depth: usize,
) {
    if depth > super::MAX_AST_DEPTH {
        tracing::warn!(
            "AST depth limit reached at {}:{}",
            path.display(),
            node.start_position().row + 1
        );
        return;
    }
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        visit_node(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            child,
            impl_type,
            depth + 1,
        );
    }
}

/// Visit a single AST node and extract IR.
#[allow(clippy::too_many_arguments)]
fn visit_node(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    impl_type: Option<&str>,
    depth: usize,
) {
    match node.kind() {
        "function_item" => {
            extract_function(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                impl_type,
                depth,
            );
        }
        "struct_item" => {
            extract_type_definition(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                SymbolKind::Struct,
            );
        }
        "enum_item" => {
            extract_type_definition(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                SymbolKind::Enum,
            );
        }
        "trait_item" => {
            extract_type_definition(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                SymbolKind::Trait,
            );
        }
        "impl_item" => {
            extract_impl_block(result, scope_stack, content, path, file_stem, node, depth);
        }
        "const_item" | "static_item" => {
            extract_constant(result, scope_stack, content, path, file_stem, node);
        }
        "use_declaration" => {
            extract_use_declaration(result, content, path, node);
        }
        "mod_item" => {
            extract_mod_item(result, scope_stack, content, path, file_stem, node, depth);
        }
        _ => {
            // Recurse into other nodes
            visit_children(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                impl_type,
                depth,
            );
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
    impl_type: Option<&str>,
    depth: usize,
) {
    let name = match extract_name(content, node) {
        Some(n) => n,
        None => return,
    };

    let visibility = extract_visibility(content, node);
    let signature = extract_signature(content, node);

    let kind = if impl_type.is_some() {
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
        visibility,
        signature,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
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

    // Visit body for nested functions (NOT as impl methods)
    if let Some(body) = node.child_by_field_name("body") {
        visit_children(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            body,
            None,
            depth + 1,
        );
    }

    scope_stack.pop();
}

/// Extract a struct, enum, or trait definition.
fn extract_type_definition(
    result: &mut ParseResult,
    scope_stack: &[usize],
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    kind: SymbolKind,
) {
    let name = match extract_name(content, node) {
        Some(n) => n,
        None => return,
    };

    let visibility = extract_visibility(content, node);
    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind,
        name,
        qualified_name,
        visibility,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    });
}

/// Extract an impl block — methods inside become `SymbolKind::Method`.
fn extract_impl_block(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    depth: usize,
) {
    let type_name = extract_impl_type_name(content, node).unwrap_or_else(|| "Unknown".to_string());

    // Push impl scope
    let impl_scope_idx = result.scopes.len();
    result.scopes.push(Scope {
        id: ScopeId::default(),
        kind: ScopeKind::Module,
        parent: None,
        name: type_name.clone(),
        location: node_location(path, node),
    });

    scope_stack.push(impl_scope_idx);

    // Visit body — functions inside become methods of this type
    if let Some(body) = node.child_by_field_name("body") {
        visit_children(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            body,
            Some(&type_name),
            depth + 1,
        );
    }

    scope_stack.pop();
}

/// Extract a const or static definition.
fn extract_constant(
    result: &mut ParseResult,
    scope_stack: &[usize],
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
) {
    let name = match extract_name(content, node) {
        Some(n) => n,
        None => return,
    };

    let visibility = extract_visibility(content, node);
    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Constant,
        name,
        qualified_name,
        visibility,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    });
}

/// Extract a `mod` item (inline module).
fn extract_mod_item(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    depth: usize,
) {
    let name = match extract_name(content, node) {
        Some(n) => n,
        None => return,
    };

    // Push module scope
    let mod_scope_idx = result.scopes.len();
    result.scopes.push(Scope {
        id: ScopeId::default(),
        kind: ScopeKind::Module,
        parent: None,
        name: name.clone(),
        location: node_location(path, node),
    });

    scope_stack.push(mod_scope_idx);

    // Visit body if it has one (inline mod with braces)
    if let Some(body) = node.child_by_field_name("body") {
        visit_children(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            body,
            None,
            depth + 1,
        );
    }

    scope_stack.pop();
}

/// Extract use declarations (imports).
///
/// Handles:
/// - `use std::collections::HashMap;` → import "HashMap"
/// - `use std::io::{Read, Write};` → imports "Read", "Write"
/// - `use X as Y;` → import "Y" (aliased)
/// - `use crate::prelude::*;` → star import
/// - `use std::io::{self, Read};` → imports "io", "Read"
fn extract_use_declaration(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let visibility = extract_visibility(content, node);

    // Walk all descendant nodes to find import targets
    extract_use_tree(result, content, path, node, visibility, None);
}

/// Recursively extract import symbols from a use declaration's tree.
///
/// The `module_path` parameter accumulates the module prefix as we descend
/// into scoped use lists. For `use crate::parser::{rust, python}`, the
/// recursive call for the `use_list` receives `Some("crate::parser")`.
fn extract_use_tree(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    node: Node,
    visibility: Visibility,
    module_path: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "use_as_clause" => {
                // `use X as Y;` — extract the alias name
                if let Some(alias_node) = child.child_by_field_name("alias") {
                    let alias_name = node_text(content, alias_node).to_string();
                    // Extract module path from the use_as_clause's path field
                    let as_module_path = child
                        .child_by_field_name("path")
                        .and_then(|p| extract_module_path(content, p))
                        .or_else(|| module_path.map(|s| s.to_string()));
                    add_import_symbol(
                        result,
                        &alias_name,
                        visibility,
                        path,
                        child,
                        as_module_path.as_deref(),
                    );
                }
            }
            "use_list" => {
                // `use std::io::{Read, Write};` — recurse into list items
                let mut list_cursor = child.walk();
                for item in child.children(&mut list_cursor) {
                    match item.kind() {
                        "identifier" => {
                            let name = node_text(content, item).to_string();
                            add_import_symbol(
                                result,
                                &name,
                                visibility,
                                path,
                                item,
                                module_path,
                            );
                        }
                        "use_as_clause" => {
                            if let Some(alias_node) = item.child_by_field_name("alias") {
                                let alias_name = node_text(content, alias_node).to_string();
                                add_import_symbol(
                                    result,
                                    &alias_name,
                                    visibility,
                                    path,
                                    item,
                                    module_path,
                                );
                            }
                        }
                        "self" => {
                            // `{self, Read}` — `self` imports the parent module
                            // Extract the parent module name from the path
                            if let Some(parent_name) = extract_use_path_last_segment(content, node)
                            {
                                add_import_symbol(
                                    result,
                                    &parent_name,
                                    visibility,
                                    path,
                                    item,
                                    module_path,
                                );
                            }
                        }
                        "scoped_identifier" => {
                            // Nested path inside use list: `use crate::foo::{bar::Baz}`
                            // Extend module_path with the scoped prefix
                            if let Some(name) = last_identifier_segment(content, item) {
                                let extended_path = if let Some(mp) = module_path {
                                    item.child_by_field_name("path")
                                        .map(|p| {
                                            format!("{}::{}", mp, build_scoped_path(content, p))
                                        })
                                        .unwrap_or_else(|| mp.to_string())
                                } else {
                                    item.child_by_field_name("path")
                                        .map(|p| build_scoped_path(content, p))
                                        .unwrap_or_default()
                                };
                                let mp = if extended_path.is_empty() {
                                    None
                                } else {
                                    Some(extended_path.as_str())
                                };
                                add_import_symbol(result, &name, visibility, path, item, mp);
                            }
                        }
                        _ => {}
                    }
                }
            }
            "use_wildcard" => {
                // `use crate::prelude::*;` — star import
                result.references.push(Reference {
                    id: ReferenceId::default(),
                    from: SymbolId::default(),
                    to: SymbolId::default(),
                    kind: ReferenceKind::Import,
                    location: node_location(path, node),
                    resolution: ResolutionStatus::Partial("star import".to_string()),
                });
            }
            "scoped_identifier" | "scoped_use_list" => {
                // This might be a simple scoped import like `use std::collections::HashMap;`
                // Check if this is the final identifier (not containing a use_list)
                if child.kind() == "scoped_identifier" && !has_use_list_descendant(child) {
                    // Final scoped identifier — extract last segment as import name
                    if let Some(name) = last_identifier_segment(content, child) {
                        // Compute module path from the scoped_identifier's path field
                        let child_module_path = extract_module_path(content, child);
                        add_import_symbol(
                            result,
                            &name,
                            visibility,
                            path,
                            child,
                            child_module_path.as_deref(),
                        );
                    }
                } else {
                    // Has nested children (scoped_use_list) — recurse with computed module path
                    let nested_module_path = child
                        .child_by_field_name("path")
                        .map(|p| build_scoped_path(content, p));
                    extract_use_tree(
                        result,
                        content,
                        path,
                        child,
                        visibility,
                        nested_module_path.as_deref(),
                    );
                }
            }
            "identifier" => {
                // Simple import: `use something;`
                // But only if this is a direct child of use_declaration, not a path segment
                if node.kind() == "use_declaration" {
                    let name = node_text(content, child).to_string();
                    add_import_symbol(result, &name, visibility, path, child, module_path);
                }
            }
            _ => {}
        }
    }
}

/// Check if a node has any `use_list` descendant.
fn has_use_list_descendant(node: Node) -> bool {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "use_list" || has_use_list_descendant(child) {
            return true;
        }
    }
    false
}

/// Extract the last identifier segment from a scoped_identifier or scoped_use_list path.
fn last_identifier_segment(content: &[u8], node: Node) -> Option<String> {
    // For scoped_identifier, the "name" field is the last segment
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(node_text(content, name_node).to_string());
    }
    // Fallback: find the last identifier child
    let mut cursor = node.walk();
    let mut last_ident = None;
    for child in node.children(&mut cursor) {
        if child.kind() == "identifier" || child.kind() == "type_identifier" {
            last_ident = Some(node_text(content, child).to_string());
        }
    }
    last_ident
}

/// Extract the last segment of the path in a use declaration (for `self` imports).
/// For `use std::io::{self, Read}`, returns "io".
fn extract_use_path_last_segment(content: &[u8], use_node: Node) -> Option<String> {
    // Walk the use_declaration's children to find the scoped path before the use_list
    let mut cursor = use_node.walk();
    for child in use_node.children(&mut cursor) {
        if child.kind() == "scoped_use_list" {
            // The path field of scoped_use_list has the prefix
            if let Some(path_node) = child.child_by_field_name("path") {
                return last_identifier_segment(content, path_node);
            }
        }
    }
    None
}

/// Recursively build the full `::` path from a scoped_identifier node.
///
/// For the tree-sitter node representing `crate::parser::rust`, returns
/// `"crate::parser::rust"`. For a simple `identifier` node, returns
/// that identifier text. Handles `crate`, `self`, `super` keywords.
fn build_scoped_path(content: &[u8], node: Node) -> String {
    match node.kind() {
        "identifier" | "type_identifier" => node_text(content, node).to_string(),
        "crate" => "crate".to_string(),
        "self" => "self".to_string(),
        "super" => "super".to_string(),
        "scoped_identifier" => {
            let path_part = node
                .child_by_field_name("path")
                .map(|n| build_scoped_path(content, n))
                .unwrap_or_default();
            let name_part = node
                .child_by_field_name("name")
                .map(|n| node_text(content, n).to_string())
                .unwrap_or_default();
            if path_part.is_empty() {
                name_part
            } else if name_part.is_empty() {
                path_part
            } else {
                format!("{}::{}", path_part, name_part)
            }
        }
        _ => node_text(content, node).to_string(),
    }
}

/// Extract the module path (everything except the last segment) from a
/// scoped_identifier node.
///
/// For `crate::parser::rust::RustAdapter`, the module path is
/// `"crate::parser::rust"` (the `path` field of the outer scoped_identifier).
/// Returns `None` for simple identifiers with no path prefix.
fn extract_module_path(content: &[u8], node: Node) -> Option<String> {
    if node.kind() == "scoped_identifier" {
        node.child_by_field_name("path")
            .map(|p| build_scoped_path(content, p))
    } else {
        None
    }
}

/// Add an import symbol to the result with optional module path annotation.
///
/// When `module_path` is provided, adds a `"from:<module>"` annotation
/// that enables `resolve_cross_file_imports()` to match this import to
/// its definition file via the module map. Matches the pattern used by
/// the Python and JavaScript adapters.
fn add_import_symbol(
    result: &mut ParseResult,
    name: &str,
    visibility: Visibility,
    path: &Path,
    node: Node,
    module_path: Option<&str>,
) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut annotations = vec!["import".to_string()];
    if let Some(mp) = module_path {
        if !mp.is_empty() {
            annotations.push(format!("from:{}", mp));
        }
    }

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Variable,
        name: name.to_string(),
        qualified_name: format!("{}::{}", file_stem, name),
        visibility,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Unresolved,
        scope: ScopeId::default(),
        annotations,
    });

    // Also create an import reference
    result.references.push(Reference {
        id: ReferenceId::default(),
        from: SymbolId::default(),
        to: SymbolId::default(),
        kind: ReferenceKind::Import,
        location: node_location(path, node),
        resolution: ResolutionStatus::Unresolved,
    });
}

/// Extract all call expressions from the AST.
///
/// Walks the entire tree looking for `call_expression` nodes. For each one,
/// extracts the callee name and creates a `ReferenceKind::Call` reference.
/// Protected by depth limit to prevent stack overflow on deeply nested expressions.
fn extract_all_calls(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    node: Node,
    depth: usize,
) {
    if depth > super::MAX_AST_DEPTH {
        tracing::warn!(
            "AST depth limit reached in call extraction at {}:{}",
            path.display(),
            node.start_position().row + 1
        );
        return;
    }

    if node.kind() == "call_expression" {
        extract_call(result, content, path, node);
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_all_calls(result, content, path, child, depth + 1);
    }
}

/// Extract a single call expression.
fn extract_call(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let callee_name = if let Some(func_node) = node.child_by_field_name("function") {
        match func_node.kind() {
            "identifier" => Some(node_text(content, func_node).to_string()),
            "scoped_identifier" => {
                // Module::function() — take last segment
                last_identifier_segment(content, func_node)
            }
            "field_expression" => {
                // object.method() — extract field name, detect self.method() pattern
                let field_name = func_node
                    .child_by_field_name("field")
                    .map(|f| node_text(content, f).to_string());

                if let Some(value_node) = func_node.child_by_field_name("value") {
                    let value_text = node_text(content, value_node);
                    if value_text == "self" || value_text == "Self" {
                        // self.method() → emit "self.<field>" to trigger resolve_callee's
                        // self-method resolution path (same pattern as Python adapter)
                        field_name.map(|f| format!("self.{}", f))
                    } else {
                        // obj.method() → emit "obj.method" so resolve_callee's dotted
                        // name check correctly marks it unresolved (type inference needed)
                        field_name.map(|f| format!("{}.{}", value_text, f))
                    }
                } else {
                    field_name
                }
            }
            _ => {
                // Fallback: use the full text
                let text = node_text(content, func_node).to_string();
                if text.is_empty() {
                    None
                } else {
                    Some(text)
                }
            }
        }
    } else {
        None
    };

    if let Some(name) = callee_name {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // =========================================================================
    // 1. can_handle() and language_name()
    // =========================================================================

    #[test]
    fn test_rust_adapter_language_name() {
        let adapter = RustAdapter::new();
        assert_eq!(adapter.language_name(), "rust");
    }

    #[test]
    fn test_rust_adapter_can_handle_rs_file() {
        let adapter = RustAdapter::new();
        assert!(adapter.can_handle(Path::new("src/main.rs")));
        assert!(adapter.can_handle(Path::new("lib.rs")));
        assert!(adapter.can_handle(Path::new("deeply/nested/module.rs")));
    }

    #[test]
    fn test_rust_adapter_rejects_non_rust_files() {
        let adapter = RustAdapter::new();
        assert!(!adapter.can_handle(Path::new("main.py")));
        assert!(!adapter.can_handle(Path::new("app.js")));
        assert!(!adapter.can_handle(Path::new("style.css")));
        assert!(!adapter.can_handle(Path::new("Cargo.toml")));
        assert!(!adapter.can_handle(Path::new("README.md")));
    }

    #[test]
    fn test_rust_adapter_rejects_no_extension() {
        let adapter = RustAdapter::new();
        assert!(!adapter.can_handle(Path::new("Makefile")));
        assert!(!adapter.can_handle(Path::new("rs"))); // "rs" without dot is NOT a .rs file
    }

    #[test]
    fn test_rust_adapter_case_sensitivity() {
        // .RS, .Rs should NOT match — Rust files are lowercase .rs
        let adapter = RustAdapter::new();
        assert!(!adapter.can_handle(Path::new("main.RS")));
        assert!(!adapter.can_handle(Path::new("main.Rs")));
    }

    // =========================================================================
    // 2. Function Extraction
    // =========================================================================

    #[test]
    fn test_rust_extract_simple_function() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("example.rs"),
                r#"
fn hello() {
    println!("hello");
}
"#,
            )
            .unwrap();

        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "hello")
            .collect();
        assert_eq!(funcs.len(), 1, "Must extract exactly one 'hello' function");
        assert_eq!(
            funcs[0].visibility,
            Visibility::Private,
            "No modifier = Private"
        );
    }

    #[test]
    fn test_rust_extract_pub_function() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("lib.rs"),
                r#"
pub fn public_fn() -> i32 { 42 }
"#,
            )
            .unwrap();

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "public_fn")
            .expect("Must find public_fn");
        assert_eq!(func.visibility, Visibility::Public);
        assert_eq!(func.kind, SymbolKind::Function);
    }

    #[test]
    fn test_rust_extract_function_with_params_and_return() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("math.rs"),
                r#"
fn add(a: i32, b: i32) -> i32 {
    a + b
}
"#,
            )
            .unwrap();

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Must find add");
        assert_eq!(func.kind, SymbolKind::Function);
        assert!(
            func.signature.is_some(),
            "Functions with parameters should have a signature"
        );
    }

    #[test]
    fn test_rust_extract_async_function() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("server.rs"),
                r#"
pub async fn handle_request() {
    // async handler
}
"#,
            )
            .unwrap();

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "handle_request")
            .expect("Must find async function");
        assert_eq!(func.kind, SymbolKind::Function);
        assert_eq!(func.visibility, Visibility::Public);
    }

    #[test]
    fn test_rust_extract_generic_function() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("generic.rs"),
                r#"
fn process<T: Display>(item: T) -> String {
    item.to_string()
}
"#,
            )
            .unwrap();

        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "process")
            .expect("Must find generic function");
        assert_eq!(
            func.name, "process",
            "Generic parameters must NOT be in the name"
        );
    }

    // =========================================================================
    // 3. Struct Extraction
    // =========================================================================

    #[test]
    fn test_rust_extract_struct() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("models.rs"),
                r#"
pub struct User {
    name: String,
    age: u32,
}
"#,
            )
            .unwrap();

        let strukt = result
            .symbols
            .iter()
            .find(|s| s.name == "User")
            .expect("Must find User struct");
        assert_eq!(strukt.kind, SymbolKind::Struct);
        assert_eq!(strukt.visibility, Visibility::Public);
    }

    #[test]
    fn test_rust_extract_tuple_struct() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("types.rs"),
                r#"
struct Point(f64, f64);
"#,
            )
            .unwrap();

        let strukt = result
            .symbols
            .iter()
            .find(|s| s.name == "Point")
            .expect("Must find tuple struct");
        assert_eq!(strukt.kind, SymbolKind::Struct);
    }

    #[test]
    fn test_rust_extract_unit_struct() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("marker.rs"),
                r#"
pub struct Marker;
"#,
            )
            .unwrap();

        let strukt = result
            .symbols
            .iter()
            .find(|s| s.name == "Marker")
            .expect("Must find unit struct");
        assert_eq!(strukt.kind, SymbolKind::Struct);
    }

    #[test]
    fn test_rust_extract_generic_struct_name_is_clean() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("container.rs"),
                r#"
pub struct Container<T: Clone + Send> {
    items: Vec<T>,
}
"#,
            )
            .unwrap();

        let strukt = result
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Struct)
            .expect("Must find generic struct");
        assert_eq!(
            strukt.name, "Container",
            "Generic params must NOT appear in name"
        );
    }

    // =========================================================================
    // 4. Enum Extraction
    // =========================================================================

    #[test]
    fn test_rust_extract_enum() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("status.rs"),
                r#"
pub enum Status {
    Active,
    Inactive,
    Pending,
}
"#,
            )
            .unwrap();

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "Status")
            .expect("Must find Status enum");
        assert_eq!(enm.kind, SymbolKind::Enum);
        assert_eq!(enm.visibility, Visibility::Public);
    }

    #[test]
    fn test_rust_extract_enum_with_data() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("result.rs"),
                r#"
enum MyResult {
    Ok(String),
    Err { code: i32, message: String },
}
"#,
            )
            .unwrap();

        let enm = result
            .symbols
            .iter()
            .find(|s| s.name == "MyResult")
            .expect("Must find enum with data variants");
        assert_eq!(enm.kind, SymbolKind::Enum);
        assert_eq!(enm.visibility, Visibility::Private);
    }

    // =========================================================================
    // 5. Trait Extraction
    // =========================================================================

    #[test]
    fn test_rust_extract_trait() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("traits.rs"),
                r#"
pub trait Drawable {
    fn draw(&self);
    fn area(&self) -> f64;
}
"#,
            )
            .unwrap();

        let trt = result
            .symbols
            .iter()
            .find(|s| s.name == "Drawable")
            .expect("Must find Drawable trait");
        assert_eq!(trt.kind, SymbolKind::Trait);
        assert_eq!(trt.visibility, Visibility::Public);
    }

    // =========================================================================
    // 6. Visibility Modifier Tests
    // =========================================================================

    #[test]
    fn test_rust_visibility_pub() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("vis.rs"),
                r#"
pub fn public_fn() {}
"#,
            )
            .unwrap();
        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "public_fn")
            .unwrap();
        assert_eq!(sym.visibility, Visibility::Public);
    }

    #[test]
    fn test_rust_visibility_pub_crate() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("vis.rs"),
                r#"
pub(crate) fn crate_fn() {}
"#,
            )
            .unwrap();
        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "crate_fn")
            .unwrap();
        assert_eq!(sym.visibility, Visibility::Crate);
    }

    #[test]
    fn test_rust_visibility_pub_super() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("vis.rs"),
                r#"
pub(super) fn super_fn() {}
"#,
            )
            .unwrap();
        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "super_fn")
            .unwrap();
        assert_eq!(sym.visibility, Visibility::Protected);
    }

    #[test]
    fn test_rust_visibility_pub_in_path() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("vis.rs"),
                r#"
pub(in crate::parent) fn restricted_fn() {}
"#,
            )
            .unwrap();
        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "restricted_fn")
            .unwrap();
        assert_eq!(
            sym.visibility,
            Visibility::Protected,
            "pub(in path) should map to Protected"
        );
    }

    #[test]
    fn test_rust_visibility_private_default() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("vis.rs"),
                r#"
fn private_fn() {}
struct PrivateStruct {}
enum PrivateEnum { A }
"#,
            )
            .unwrap();
        for sym in &result.symbols {
            if sym.name.starts_with("Private") || sym.name == "private_fn" {
                assert_eq!(
                    sym.visibility,
                    Visibility::Private,
                    "No modifier must be Private for '{}'",
                    sym.name
                );
            }
        }
    }

    #[test]
    fn test_rust_visibility_on_struct_fields_ignored() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("fields.rs"),
                r#"
pub struct Config {
    pub name: String,
    pub(crate) secret: String,
    internal: u32,
}
"#,
            )
            .unwrap();

        let structs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .collect();
        assert_eq!(structs.len(), 1, "Only the struct itself, not its fields");
        assert_eq!(structs[0].name, "Config");
    }

    // =========================================================================
    // 7. Impl Block Association (P0)
    // =========================================================================

    #[test]
    fn test_rust_impl_method_is_method_kind() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("service.rs"),
                r#"
struct UserService;

impl UserService {
    pub fn new() -> Self { Self }
    pub fn get_user(&self, id: u64) -> Option<u64> { None }
    fn internal_helper(&self) {}
}
"#,
            )
            .unwrap();

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 3,
            "All functions in impl block must be Methods, got {}",
            methods.len()
        );

        for m in &methods {
            assert!(
                m.qualified_name.contains("UserService"),
                "Method '{}' qualified_name must contain 'UserService', got '{}'",
                m.name,
                m.qualified_name
            );
        }
    }

    #[test]
    fn test_rust_impl_method_visibility() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("impl_vis.rs"),
                r#"
struct Foo;
impl Foo {
    pub fn public_method(&self) {}
    fn private_method(&self) {}
    pub(crate) fn crate_method(&self) {}
}
"#,
            )
            .unwrap();

        let pub_m = result
            .symbols
            .iter()
            .find(|s| s.name == "public_method")
            .unwrap();
        assert_eq!(pub_m.visibility, Visibility::Public);
        assert_eq!(pub_m.kind, SymbolKind::Method);

        let priv_m = result
            .symbols
            .iter()
            .find(|s| s.name == "private_method")
            .unwrap();
        assert_eq!(priv_m.visibility, Visibility::Private);

        let crate_m = result
            .symbols
            .iter()
            .find(|s| s.name == "crate_method")
            .unwrap();
        assert_eq!(crate_m.visibility, Visibility::Crate);
    }

    #[test]
    fn test_rust_impl_trait_for_type() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("display.rs"),
                r#"
struct Point { x: f64, y: f64 }

impl std::fmt::Display for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "({}, {})", self.x, self.y)
    }
}
"#,
            )
            .unwrap();

        let fmt_method = result
            .symbols
            .iter()
            .find(|s| s.name == "fmt" && s.kind == SymbolKind::Method)
            .expect("fmt must be extracted as a Method from trait impl");
        assert!(
            fmt_method.qualified_name.contains("Point"),
            "Trait impl methods should be qualified with the implementing type, got '{}'",
            fmt_method.qualified_name
        );
    }

    #[test]
    fn test_rust_multiple_impl_blocks_same_type() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("multi_impl.rs"),
                r#"
struct Foo;

impl Foo {
    fn method_a(&self) {}
}

impl Foo {
    fn method_b(&self) {}
}
"#,
            )
            .unwrap();

        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert_eq!(
            methods.len(),
            2,
            "Both impl blocks' methods must be extracted"
        );

        for m in &methods {
            assert!(
                m.qualified_name.contains("Foo"),
                "Method '{}' from either impl block must be associated with Foo",
                m.name
            );
        }
    }

    #[test]
    fn test_rust_impl_generic_type() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("generic_impl.rs"),
                r#"
struct Container<T> { items: Vec<T> }

impl<T> Container<T> {
    fn push(&mut self, item: T) {
        self.items.push(item);
    }
}
"#,
            )
            .unwrap();

        let push = result
            .symbols
            .iter()
            .find(|s| s.name == "push" && s.kind == SymbolKind::Method)
            .expect("Must find push method in generic impl");
        assert!(
            push.qualified_name.contains("Container"),
            "Generic impl should use base type name 'Container', got '{}'",
            push.qualified_name
        );
        assert!(
            !push.qualified_name.contains("<T>"),
            "Qualified name should not contain generic params, got '{}'",
            push.qualified_name
        );
    }

    #[test]
    fn test_rust_standalone_fn_not_method() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("mixed.rs"),
                r#"
fn top_level() {}

struct S;
impl S {
    fn method() {}
}

fn another_top_level() {}
"#,
            )
            .unwrap();

        let top = result
            .symbols
            .iter()
            .find(|s| s.name == "top_level")
            .unwrap();
        assert_eq!(
            top.kind,
            SymbolKind::Function,
            "Top-level fn must be Function, not Method"
        );

        let another = result
            .symbols
            .iter()
            .find(|s| s.name == "another_top_level")
            .unwrap();
        assert_eq!(another.kind, SymbolKind::Function);

        let method = result.symbols.iter().find(|s| s.name == "method").unwrap();
        assert_eq!(method.kind, SymbolKind::Method);
    }

    // =========================================================================
    // 8. Use Statement Extraction
    // =========================================================================

    #[test]
    fn test_rust_simple_use() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("imports.rs"),
                r#"
use std::collections::HashMap;
"#,
            )
            .unwrap();

        let import = result
            .symbols
            .iter()
            .find(|s| s.name == "HashMap" && s.annotations.contains(&"import".to_string()))
            .expect("Must extract HashMap as import symbol");
        assert!(import.annotations.contains(&"import".to_string()));
    }

    #[test]
    fn test_rust_use_list() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("imports.rs"),
                r#"
use std::io::{Read, Write, BufRead};
"#,
            )
            .unwrap();

        let imports: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"import".to_string()))
            .collect();
        let names: Vec<&str> = imports.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Read"), "Must extract Read from use list");
        assert!(names.contains(&"Write"), "Must extract Write from use list");
        assert!(
            names.contains(&"BufRead"),
            "Must extract BufRead from use list"
        );
    }

    #[test]
    fn test_rust_use_alias() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("imports.rs"),
                r#"
use std::path::Path as FilePath;
"#,
            )
            .unwrap();

        let import = result
            .symbols
            .iter()
            .find(|s| s.name == "FilePath" && s.annotations.contains(&"import".to_string()))
            .expect("Must extract aliased import as 'FilePath'");
        assert_eq!(
            import.name, "FilePath",
            "Alias name should be the symbol name"
        );
    }

    #[test]
    fn test_rust_use_star() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("imports.rs"),
                r#"
use crate::prelude::*;
"#,
            )
            .unwrap();

        let refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(
            !refs.is_empty(),
            "Star import must produce at least one import reference"
        );
    }

    #[test]
    fn test_rust_use_self_in_list() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("imports.rs"),
                r#"
use std::io::{self, Read};
"#,
            )
            .unwrap();

        let imports: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"import".to_string()))
            .collect();
        let has_read = imports.iter().any(|s| s.name == "Read");
        assert!(has_read, "Must extract 'Read' from use list with self");
    }

    #[test]
    fn test_rust_use_crate_and_super() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("sub/mod.rs"),
                r#"
use crate::config::Config;
use super::parent_fn;
"#,
            )
            .unwrap();

        let imports: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"import".to_string()))
            .collect();
        let names: Vec<&str> = imports.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Config"),
            "Must extract crate-relative import"
        );
        assert!(
            names.contains(&"parent_fn"),
            "Must extract super-relative import"
        );
    }

    #[test]
    fn test_rust_pub_use_reexport() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("lib.rs"),
                r#"
pub use crate::module::Type;
"#,
            )
            .unwrap();

        let sym = result
            .symbols
            .iter()
            .find(|s| s.name == "Type")
            .expect("Must extract pub use as a symbol");
        assert_eq!(
            sym.visibility,
            Visibility::Public,
            "pub use re-export must have Public visibility"
        );
    }

    // =========================================================================
    // 9. Call-Site Detection
    // =========================================================================

    #[test]
    fn test_rust_function_call_detection() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("calls.rs"),
                r#"
fn helper() -> i32 { 42 }

fn main() {
    let x = helper();
}
"#,
            )
            .unwrap();

        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Call)
            .collect();
        assert!(!calls.is_empty(), "Must detect function call to helper()");
    }

    #[test]
    fn test_rust_method_call_detection() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("method_calls.rs"),
                r#"
fn process(data: Vec<String>) {
    let result = data.iter().map(|s| s.len()).collect::<Vec<_>>();
}
"#,
            )
            .unwrap();

        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Call)
            .collect();
        assert!(
            !calls.is_empty(),
            "Must detect method calls (.iter(), .map(), .collect())"
        );
    }

    #[test]
    fn test_rust_associated_function_call() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("assoc.rs"),
                r#"
fn create() {
    let map = HashMap::new();
    let s = String::from("hello");
}
"#,
            )
            .unwrap();

        let calls: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Call)
            .collect();
        assert!(
            calls.len() >= 2,
            "Must detect associated function calls (::new(), ::from())"
        );
    }

    // =========================================================================
    // 10. Adversarial Tests
    // =========================================================================

    #[test]
    fn test_rust_empty_file() {
        let adapter = RustAdapter::new();
        let result = adapter.parse_file(Path::new("empty.rs"), "").unwrap();
        assert!(
            result.symbols.is_empty(),
            "Empty file must produce zero symbols"
        );
        assert!(
            result.references.is_empty(),
            "Empty file must produce zero references"
        );
    }

    #[test]
    fn test_rust_comment_only_file() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("comments.rs"),
                r#"
// This file has only comments.
/* Block comment */
/// Doc comment without any code
"#,
            )
            .unwrap();
        assert!(
            result.symbols.is_empty(),
            "Comment-only file must produce zero symbols"
        );
    }

    #[test]
    fn test_rust_syntax_error_file() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("broken.rs"),
                r#"
fn valid_fn() -> i32 { 42 }

fn broken( {
    // syntax error: missing closing paren and type
}

struct GoodStruct {
    field: i32,
}
"#,
            )
            .unwrap();

        let valid = result.symbols.iter().find(|s| s.name == "valid_fn");
        assert!(
            valid.is_some(),
            "Must extract valid_fn despite syntax errors elsewhere"
        );
        let good_struct = result.symbols.iter().find(|s| s.name == "GoodStruct");
        assert!(
            good_struct.is_some(),
            "Must extract GoodStruct despite syntax errors elsewhere"
        );
    }

    #[test]
    fn test_rust_nested_function_inside_function() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("nested.rs"),
                r#"
fn outer() {
    fn inner() {
        println!("nested");
    }
    inner();
}
"#,
            )
            .unwrap();

        let inner = result.symbols.iter().find(|s| s.name == "inner").unwrap();
        assert_eq!(
            inner.kind,
            SymbolKind::Function,
            "Nested function inside a function is still Function, not Method"
        );
    }

    #[test]
    fn test_rust_deeply_nested_impl() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("deep.rs"),
                r#"
mod inner {
    pub struct Deep;
    impl Deep {
        pub fn method(&self) {}
    }
}
"#,
            )
            .unwrap();

        let method = result
            .symbols
            .iter()
            .find(|s| s.name == "method" && s.kind == SymbolKind::Method);
        assert!(
            method.is_some(),
            "Must extract method from impl inside inline mod"
        );
    }

    #[test]
    fn test_rust_const_and_static() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("consts.rs"),
                r#"
pub const MAX_SIZE: usize = 1024;
static COUNTER: u32 = 0;
"#,
            )
            .unwrap();

        let max_size = result.symbols.iter().find(|s| s.name == "MAX_SIZE");
        assert!(max_size.is_some(), "Must extract const");
        assert_eq!(max_size.unwrap().kind, SymbolKind::Constant);

        let counter = result.symbols.iter().find(|s| s.name == "COUNTER");
        assert!(counter.is_some(), "Must extract static");
        assert_eq!(counter.unwrap().kind, SymbolKind::Constant);
    }

    #[test]
    fn test_rust_cfg_test_module() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("with_tests.rs"),
                r#"
pub fn production_code() -> bool { true }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_production_code() {
        assert!(production_code());
    }
}
"#,
            )
            .unwrap();

        let prod = result.symbols.iter().find(|s| s.name == "production_code");
        assert!(prod.is_some(), "Production function must be extracted");
    }

    #[test]
    fn test_rust_qualified_name_format() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("module.rs"),
                r#"
fn top_level() {}

struct MyStruct;
impl MyStruct {
    fn method() {}
}
"#,
            )
            .unwrap();

        let top = result
            .symbols
            .iter()
            .find(|s| s.name == "top_level")
            .unwrap();
        assert!(
            top.qualified_name.contains("module.rs"),
            "Qualified name should contain the file name, got '{}'",
            top.qualified_name
        );

        let method = result.symbols.iter().find(|s| s.name == "method").unwrap();
        assert!(
            method.qualified_name.contains("MyStruct"),
            "Method qualified name should contain struct name, got '{}'",
            method.qualified_name
        );
    }

    #[test]
    fn test_rust_location_accuracy() {
        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(Path::new("loc.rs"), "fn first() {}\n\n\nfn fourth() {}\n")
            .unwrap();

        let first = result.symbols.iter().find(|s| s.name == "first").unwrap();
        assert_eq!(first.location.line, 1, "first() should be on line 1");

        let fourth = result.symbols.iter().find(|s| s.name == "fourth").unwrap();
        assert_eq!(fourth.location.line, 4, "fourth() should be on line 4");
    }

    // =========================================================================
    // 11. Fixture File Test — Multi-Construct Rust File
    // =========================================================================

    #[test]
    fn test_rust_fixture_multi_construct_extraction() {
        let adapter = RustAdapter::new();
        let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let fixture_path = manifest
            .parent()
            .unwrap()
            .join("tests/fixtures/rust/sample.rs");
        let content = std::fs::read_to_string(&fixture_path)
            .unwrap_or_else(|_| panic!("Rust fixture not found at {:?}", fixture_path));
        let result = adapter
            .parse_file(Path::new("sample.rs"), &content)
            .unwrap();

        // At least 1 Function
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.kind == SymbolKind::Function),
            "Must have at least 1 Function"
        );
        // At least 1 Struct
        assert!(
            result.symbols.iter().any(|s| s.kind == SymbolKind::Struct),
            "Must have at least 1 Struct"
        );
        // At least 1 Enum
        assert!(
            result.symbols.iter().any(|s| s.kind == SymbolKind::Enum),
            "Must have at least 1 Enum"
        );
        // At least 1 Trait
        assert!(
            result.symbols.iter().any(|s| s.kind == SymbolKind::Trait),
            "Must have at least 1 Trait"
        );
        // At least 1 Method (from impl block)
        assert!(
            result.symbols.iter().any(|s| s.kind == SymbolKind::Method),
            "Must have at least 1 Method"
        );
        // At least 1 Constant
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.kind == SymbolKind::Constant),
            "Must have at least 1 Constant"
        );
        // At least 1 import symbol
        assert!(
            result
                .symbols
                .iter()
                .any(|s| s.annotations.contains(&"import".to_string())),
            "Must have at least 1 import symbol"
        );
        // At least 1 Call reference
        assert!(
            result
                .references
                .iter()
                .any(|r| r.kind == ReferenceKind::Call),
            "Must have at least 1 Call reference"
        );
        // All symbols have non-empty qualified_names
        for sym in &result.symbols {
            assert!(
                !sym.qualified_name.is_empty(),
                "Symbol '{}' must have non-empty qualified_name",
                sym.name
            );
        }
        // All symbols have valid locations (line > 0)
        for sym in &result.symbols {
            assert!(
                sym.location.line > 0,
                "Symbol '{}' must have line > 0",
                sym.name
            );
        }
    }

    // =========================================================================
    // 12. Integration — Populate Graph
    // =========================================================================

    #[test]
    fn test_rust_parse_result_populates_graph() {
        use crate::graph::{populate_graph, Graph};

        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("sample.rs"),
                r#"
pub fn hello() {}
struct Data { x: i32 }
"#,
            )
            .unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        let symbols: Vec<_> = graph.all_symbols().collect();
        assert!(
            symbols.len() >= 2,
            "populate_graph must add Rust symbols to graph"
        );
    }

    #[test]
    fn test_rust_adapter_graph_edges_are_queryable() {
        use crate::graph::{populate_graph, Graph};

        let adapter = RustAdapter::new();
        let result = adapter
            .parse_file(
                Path::new("edge_test.rs"),
                r#"
fn helper() -> i32 { 42 }

fn main() {
    let x = helper();
}
"#,
            )
            .unwrap();

        let mut graph = Graph::new();
        populate_graph(&mut graph, &result);

        // Verify call references were created
        let call_refs: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Call)
            .collect();
        assert!(!call_refs.is_empty(), "Must have call references");
    }

    // =========================================================================
    // 13. Regression Tests
    // =========================================================================

    #[test]
    fn test_regression_make_symbol_trims_rs_extension() {
        use crate::test_utils::make_symbol;
        let sym = make_symbol(
            "func",
            SymbolKind::Function,
            Visibility::Public,
            "module.rs",
            1,
        );
        assert_eq!(
            sym.qualified_name, "module::func",
            "test_utils make_symbol must trim .rs from qualified_name"
        );
    }

    // =========================================================================
    // QA-2: Recursion Depth Protection — Rust Adapter (Cycle 7)
    // =========================================================================

    #[test]
    fn test_rust_deeply_nested_impl_blocks_no_crash() {
        let adapter = RustAdapter::new();
        let mut code = String::new();
        for i in 0..300 {
            code.push_str(&format!("struct S{i}; impl S{i} {{ fn m{i}() {{ "));
        }
        for _ in 0..300 {
            code.push_str("} } ");
        }
        let path = Path::new("deep_impl.rs");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on deeply nested impl blocks"
        );
        let parsed = result.unwrap();
        assert!(
            !parsed.symbols.is_empty(),
            "Must extract symbols before depth limit"
        );
        let struct_count = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Struct)
            .count();
        assert!(
            struct_count > 0,
            "Structs at shallow depth must be extracted"
        );
    }

    #[test]
    fn test_rust_deeply_nested_mod_blocks_no_crash() {
        let adapter = RustAdapter::new();
        let mut code = String::new();
        for i in 0..500 {
            code.push_str(&format!("mod m{i} {{ "));
        }
        code.push_str("fn innermost() {}");
        for _ in 0..500 {
            code.push_str(" }");
        }
        let path = Path::new("deep_mod.rs");
        let result = adapter.parse_file(path, &code);
        assert!(result.is_ok(), "Must not crash on 500-deep mod nesting");
    }

    #[test]
    fn test_rust_10k_expression_nesting_no_crash() {
        let adapter = RustAdapter::new();
        let inner = "1 + ".repeat(10_000);
        let code = format!("fn f() {{ let x = {}1; }}", inner);
        let path = Path::new("deep_expr.rs");
        let result = adapter.parse_file(path, &code);
        assert!(result.is_ok(), "Must not crash on 10K-deep expression tree");
        let parsed = result.unwrap();
        let has_fn = parsed.symbols.iter().any(|s| s.name == "f");
        assert!(
            has_fn,
            "Top-level function must be extracted even when expressions are too deep"
        );
    }

    #[test]
    fn test_rust_deeply_nested_match_no_crash() {
        let adapter = RustAdapter::new();
        let mut code = String::from("fn f(x: i32) { ");
        for _ in 0..400 {
            code.push_str("match x { 0 => { ");
        }
        code.push_str("()");
        for _ in 0..400 {
            code.push_str(" }, _ => () }");
        }
        code.push_str(" }");
        let path = Path::new("deep_match.rs");
        let result = adapter.parse_file(path, &code);
        assert!(result.is_ok(), "Must not crash on 400-deep match nesting");
    }

    #[test]
    fn test_rust_partial_results_before_depth_limit() {
        let adapter = RustAdapter::new();
        let shallow_fn = "pub fn shallow_top() {}\n";
        let inner = "1 + ".repeat(10_000);
        let deep_fn = format!("fn deep_container() {{ let x = {}1; }}\n", inner);
        let code = format!("{}{}", shallow_fn, deep_fn);

        let path = Path::new("partial.rs");
        let result = adapter.parse_file(path, &code).unwrap();

        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"shallow_top"),
            "Shallow function must be extracted"
        );
        assert!(
            names.contains(&"deep_container"),
            "Deep container function must be extracted"
        );
    }

    #[test]
    fn test_rust_deep_nesting_exit_code_not_sigabrt() {
        let adapter = RustAdapter::new();
        let inner = "1 + ".repeat(5_000);
        let code = format!("fn f() {{ let x = {}1; }}", inner);
        let path = Path::new("no_abort.rs");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Deep nesting must produce Ok result, not crash"
        );
    }
}
