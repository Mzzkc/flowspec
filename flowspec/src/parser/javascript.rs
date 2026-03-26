// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! JavaScript/TypeScript language adapter using tree-sitter-javascript.
//!
//! Extracts functions (named, arrow, async, generator), classes, methods,
//! and imports from JavaScript and TypeScript source files. Builds scope
//! hierarchy. Detects `export` for visibility (`export` = public, no
//! `export` = private).
//!
//! TypeScript files (`.ts`, `.tsx`) are accepted and parsed with the
//! JavaScript grammar as a stopgap — `tree-sitter-typescript` 0.23.2 is
//! incompatible with `tree-sitter = "0.25"`. Basic TS parses correctly;
//! generics, interfaces, and type annotations may produce parse errors
//! but tree-sitter is error-tolerant.

use std::path::Path;

use tree_sitter::{Node, Parser};

use super::ir::*;
use super::LanguageAdapter;
use crate::error::FlowspecError;

/// JavaScript/TypeScript language adapter backed by tree-sitter-javascript.
///
/// Implements the [`LanguageAdapter`] trait. Creates a fresh tree-sitter
/// parser per `parse_file` call (parsers are not `Send`). Accepts `.js`,
/// `.jsx`, `.ts`, `.tsx`, and `.mjs` file extensions.
pub struct JsAdapter;

impl JsAdapter {
    /// Creates a new JavaScript adapter.
    pub fn new() -> Self {
        Self
    }
}

impl Default for JsAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for JsAdapter {
    fn language_name(&self) -> &str {
        "javascript"
    }

    fn can_handle(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| matches!(e, "js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs"))
            .unwrap_or(false)
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError> {
        let is_ts = is_typescript_file(path);
        let mut ts_entities: Vec<Symbol> = Vec::new();
        let parse_content = if is_ts {
            let fs = path
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string());
            ts_entities = pre_extract_ts_entities(content, path, &fs);
            preprocess_typescript(content)
        } else {
            content.to_string()
        };

        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|e| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: format!("failed to set JavaScript language: {}", e),
            })?;

        let tree = parser
            .parse(&parse_content, None)
            .ok_or_else(|| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: "tree-sitter failed to produce a parse tree".to_string(),
            })?;

        let content_bytes = parse_content.as_bytes();
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
            false, // not inside export
            0,
        );

        // Extract all function/method calls from the AST
        extract_all_calls(&mut result, content_bytes, path, root, 0);

        result.symbols.extend(ts_entities);

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

/// Visit all children of a node and extract IR.
#[allow(clippy::too_many_arguments)]
fn visit_children(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
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
            exported,
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
    exported: bool,
    depth: usize,
) {
    match node.kind() {
        "function_declaration" | "generator_function_declaration" => {
            extract_function_declaration(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                exported,
                depth,
            );
        }
        "class_declaration" => {
            extract_class_declaration(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                exported,
                depth,
            );
        }
        "export_statement" => {
            visit_export_statement(result, scope_stack, content, path, file_stem, node, depth);
        }
        "lexical_declaration" | "variable_declaration" => {
            extract_arrow_functions_from_declaration(
                result,
                scope_stack,
                content,
                path,
                file_stem,
                node,
                exported,
                depth,
            );
        }
        "import_statement" => {
            extract_import(result, content, path, node);
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
                false,
                depth,
            );
        }
    }
}

/// Handle `export_statement` — sets exported flag and visits inner declarations.
fn visit_export_statement(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    depth: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "function_declaration" | "generator_function_declaration" => {
                extract_function_declaration(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    true,
                    depth,
                );
            }
            "class_declaration" => {
                extract_class_declaration(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    true,
                    depth,
                );
            }
            "lexical_declaration" | "variable_declaration" => {
                extract_arrow_functions_from_declaration(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    true,
                    depth,
                );
            }
            "export_clause" => {
                // `export { foo, bar }` or `export { foo as bar }`
                // Check for re-exports: `export { foo } from './module'` — has a string source
                let source_module = extract_source_module(content, node);
                if let Some(ref source) = source_module {
                    // Re-export: create import symbols for the re-exported names
                    extract_reexport_clause(result, content, path, file_stem, child, source);
                } else {
                    apply_export_clause_visibility(result, content, child);
                }
            }
            "*" => {
                // `export * from './module'` — star re-export
                // Extract source and create a star import symbol
                let source_module = extract_source_module(content, node);
                if let Some(ref source) = source_module {
                    let star_name = format!("*:{}", source);
                    add_import_symbol(
                        result,
                        file_stem,
                        &star_name,
                        None,
                        Some(source),
                        path,
                        node,
                    );
                }
            }
            _ => {}
        }
    }
}

/// Extract re-exported names from `export { foo, bar } from './module'`.
///
/// Creates import symbols for each re-exported name with `"from:<module>"`
/// annotations so that cross-file resolution can trace the chain.
fn extract_reexport_clause(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    clause_node: Node,
    source_module: &str,
) {
    let mut cursor = clause_node.walk();
    for child in clause_node.children(&mut cursor) {
        if child.kind() == "export_specifier" {
            let (sym_name, original_name) = extract_specifier_names(content, child);
            if !sym_name.is_empty() {
                // For re-exports, the "name" field is the source name and
                // "alias" is the exported-as name. Use original_name for lookup.
                add_import_symbol(
                    result,
                    file_stem,
                    &sym_name,
                    original_name.as_deref(),
                    Some(source_module),
                    path,
                    child,
                );
                // Also mark as public since it's exported
                for sym in result.symbols.iter_mut().rev() {
                    if sym.name == sym_name && sym.annotations.contains(&"import".to_string()) {
                        sym.visibility = Visibility::Public;
                        break;
                    }
                }
            }
        }
    }
}

/// Updates visibility to `Public` for symbols named in an `export_clause`.
///
/// Handles `export { foo }` and `export { foo as bar }` syntax. For each
/// `export_specifier`, extracts the local name (the `name` field) and updates
/// the matching symbol's visibility. Unknown names are silently skipped.
fn apply_export_clause_visibility(result: &mut ParseResult, content: &[u8], clause_node: Node) {
    let mut cursor = clause_node.walk();
    for child in clause_node.children(&mut cursor) {
        if child.kind() == "export_specifier" {
            // The local name is the `name` field; `alias` is the exported-as name
            let local_name = child
                .child_by_field_name("name")
                .map(|n| node_text(content, n).to_string())
                .unwrap_or_default();

            if local_name.is_empty() {
                continue;
            }

            // Find the symbol with this name and update its visibility
            for sym in result.symbols.iter_mut() {
                if sym.name == local_name {
                    sym.visibility = Visibility::Public;
                    break;
                }
            }
        }
    }
}

/// Extract a function declaration (named, async, generator).
#[allow(clippy::too_many_arguments)]
fn extract_function_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
    depth: usize,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(content, n).to_string())
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    let params = node
        .child_by_field_name("parameters")
        .map(|n| collapse_signature_whitespace(node_text(content, n)));

    let visibility = if exported {
        Visibility::Public
    } else {
        Visibility::Private
    };

    // Determine if method (parent scope is a class)
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
        visibility,
        signature: params,
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

    // Visit body
    if let Some(body) = node.child_by_field_name("body") {
        visit_children(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            body,
            false,
            depth + 1,
        );
    }

    scope_stack.pop();
}

/// Extract arrow functions from `lexical_declaration` / `variable_declaration`.
///
/// Walks `variable_declarator` children. If the value field is an `arrow_function`,
/// extracts it as a `SymbolKind::Function` with the variable name.
#[allow(clippy::too_many_arguments)]
fn extract_arrow_functions_from_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
    depth: usize,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "variable_declarator" {
            let name = child
                .child_by_field_name("name")
                .map(|n| node_text(content, n).to_string())
                .unwrap_or_default();

            let value = child.child_by_field_name("value");

            if let Some(value_node) = value {
                if value_node.kind() == "arrow_function" {
                    if name.is_empty() {
                        continue;
                    }

                    let params = value_node
                        .child_by_field_name("parameters")
                        .map(|n| collapse_signature_whitespace(node_text(content, n)));

                    // For single-param arrows without parens, the parameter field
                    // might be an identifier directly
                    let params = params.or_else(|| {
                        value_node
                            .child_by_field_name("parameter")
                            .map(|n| node_text(content, n).to_string())
                    });

                    let visibility = if exported {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    let qualified_name =
                        build_qualified_name(result, scope_stack, file_stem, &name);

                    result.symbols.push(Symbol {
                        id: SymbolId::default(),
                        kind: SymbolKind::Function,
                        name: name.clone(),
                        qualified_name,
                        visibility,
                        signature: params,
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
                        location: node_location(path, value_node),
                    });

                    scope_stack.push(func_scope_idx);

                    if let Some(body) = value_node.child_by_field_name("body") {
                        visit_children(
                            result,
                            scope_stack,
                            content,
                            path,
                            file_stem,
                            body,
                            false,
                            depth + 1,
                        );
                    }

                    scope_stack.pop();
                } else if value_node.kind() == "function"
                    || value_node.kind() == "generator_function"
                {
                    // `const foo = function() {}` or `const foo = function*() {}`
                    if name.is_empty() {
                        continue;
                    }

                    let params = value_node
                        .child_by_field_name("parameters")
                        .map(|n| collapse_signature_whitespace(node_text(content, n)));

                    let visibility = if exported {
                        Visibility::Public
                    } else {
                        Visibility::Private
                    };

                    let qualified_name =
                        build_qualified_name(result, scope_stack, file_stem, &name);

                    result.symbols.push(Symbol {
                        id: SymbolId::default(),
                        kind: SymbolKind::Function,
                        name: name.clone(),
                        qualified_name,
                        visibility,
                        signature: params,
                        location: node_location(path, node),
                        resolution: ResolutionStatus::Resolved,
                        scope: ScopeId::default(),
                        annotations: vec![],
                    });

                    let func_scope_idx = result.scopes.len();
                    result.scopes.push(Scope {
                        id: ScopeId::default(),
                        kind: ScopeKind::Function,
                        parent: None,
                        name: name.clone(),
                        location: node_location(path, value_node),
                    });

                    scope_stack.push(func_scope_idx);

                    if let Some(body) = value_node.child_by_field_name("body") {
                        visit_children(
                            result,
                            scope_stack,
                            content,
                            path,
                            file_stem,
                            body,
                            false,
                            depth + 1,
                        );
                    }

                    scope_stack.pop();
                }
            }
        }
    }
}

/// Extract a class declaration.
#[allow(clippy::too_many_arguments)]
fn extract_class_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
    depth: usize,
) {
    let name = node
        .child_by_field_name("name")
        .map(|n| node_text(content, n).to_string())
        .unwrap_or_default();

    if name.is_empty() {
        return;
    }

    let visibility = if exported {
        Visibility::Public
    } else {
        Visibility::Private
    };

    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Class,
        name: name.clone(),
        qualified_name,
        visibility,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations: vec![],
    });

    // Class scope (Module kind — classes define a namespace, same as PythonAdapter)
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
            if child.kind() == "method_definition" {
                extract_method_definition(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    depth + 1,
                );
            } else {
                visit_node(
                    result,
                    scope_stack,
                    content,
                    path,
                    file_stem,
                    child,
                    false,
                    depth + 1,
                );
            }
        }
    }

    scope_stack.pop();
}

/// Extract a method definition inside a class body.
///
/// Detects getter/setter keywords (`get value()`, `set value(v)`) and prefixes
/// the name with `get_`/`set_` to produce distinct entity IDs. Regular methods
/// named `getUser` or `setValue` are NOT affected — only the `get`/`set`
/// keyword syntax produces prefixed names.
fn extract_method_definition(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    depth: usize,
) {
    let raw_name = node
        .child_by_field_name("name")
        .map(|n| node_text(content, n).to_string())
        .unwrap_or_default();

    if raw_name.is_empty() {
        return;
    }

    // Detect getter/setter keyword children before the name field.
    // For `get value()`, tree-sitter produces: "get" keyword, then property_identifier "value".
    // For regular `getUser()`, there is no separate keyword — just property_identifier "getUser".
    let mut is_getter = false;
    let mut is_setter = false;
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        // Stop once we hit the name field — keyword comes before name
        if child.kind() == "property_identifier" || child.kind() == "computed_property_name" {
            break;
        }
        let text = node_text(content, child);
        if text == "get" {
            is_getter = true;
        } else if text == "set" {
            is_setter = true;
        }
    }

    let name = if is_getter {
        format!("get_{}", raw_name)
    } else if is_setter {
        format!("set_{}", raw_name)
    } else {
        raw_name
    };

    let mut annotations = vec![];
    if is_getter {
        annotations.push("getter".to_string());
    }
    if is_setter {
        annotations.push("setter".to_string());
    }

    let params = node
        .child_by_field_name("parameters")
        .map(|n| collapse_signature_whitespace(node_text(content, n)));

    let qualified_name = build_qualified_name(result, scope_stack, file_stem, &name);

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Method,
        name: name.clone(),
        qualified_name,
        visibility: Visibility::Public, // Methods in JS classes are public by default
        signature: params,
        location: node_location(path, node),
        resolution: ResolutionStatus::Resolved,
        scope: ScopeId::default(),
        annotations,
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

    if let Some(body) = node.child_by_field_name("body") {
        visit_children(
            result,
            scope_stack,
            content,
            path,
            file_stem,
            body,
            false,
            depth + 1,
        );
    }

    scope_stack.pop();
}

/// Extracts the source module path from an import/export statement's `string` child.
///
/// For `import { foo } from './bar'`, the tree-sitter AST contains a `string` child
/// node with value `'./bar'`. This function finds it and strips the surrounding quotes.
fn extract_source_module(content: &[u8], node: Node) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(content, child);
            // Strip surrounding quotes (' or ")
            let trimmed = raw
                .trim_start_matches(['\'', '"'])
                .trim_end_matches(['\'', '"']);
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Extract ES6 import statements.
///
/// Creates both a `Reference` (for graph edges) and a `Symbol` (for pattern detection)
/// for each imported name. The symbol uses `SymbolKind::Module` with an `"import"`
/// annotation so that `phantom_dependency` can find and check import symbols.
/// Also extracts the source module path as a `"from:<module>"` annotation for
/// cross-file resolution.
fn extract_import(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    // Extract source module from the string child (e.g., './bar' from `import { x } from './bar'`)
    let source_module = extract_source_module(content, node);
    let source_ref = source_module.as_deref();

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `import x from 'y'` — default import
            "identifier" => {
                let name = node_text(content, child).to_string();
                if !name.is_empty() && name != "import" && name != "from" {
                    add_import_symbol(result, &file_stem, &name, None, source_ref, path, node);
                }
            }
            // `import { a, b } from 'y'` — named imports
            "import_clause" => {
                extract_import_clause(result, content, path, &file_stem, child, source_ref);
            }
            // `import * as x from 'y'` — namespace import
            "namespace_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(content, name_node).to_string();
                    if !name.is_empty() {
                        add_import_symbol(result, &file_stem, &name, None, source_ref, path, node);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extracts the local name and original name from an `import_specifier`.
///
/// For `import { foo as bar }`, returns `("bar", Some("foo"))`.
/// For `import { foo }`, returns `("foo", None)`.
fn extract_specifier_names(content: &[u8], specifier: Node) -> (String, Option<String>) {
    let alias_node = specifier.child_by_field_name("alias");
    let name_node = specifier.child_by_field_name("name");

    match (alias_node, name_node) {
        (Some(alias), Some(name)) => {
            let alias_text = node_text(content, alias).to_string();
            let name_text = node_text(content, name).to_string();
            (alias_text, Some(name_text))
        }
        (None, Some(name)) => {
            let name_text = node_text(content, name).to_string();
            (name_text, None)
        }
        _ => (String::new(), None),
    }
}

/// Extract named imports from an import clause `{ a, b as c }`.
///
/// Passes the `source_module` from the parent `import_statement` to each
/// `add_import_symbol` call, and extracts `original_name` for aliased imports.
fn extract_import_clause(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    source_module: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(content, child).to_string();
                if !name.is_empty() && name != "import" && name != "from" {
                    add_import_symbol(result, file_stem, &name, None, source_module, path, child);
                }
            }
            "import_specifier" => {
                let (sym_name, original_name) = extract_specifier_names(content, child);
                if !sym_name.is_empty() {
                    add_import_symbol(
                        result,
                        file_stem,
                        &sym_name,
                        original_name.as_deref(),
                        source_module,
                        path,
                        child,
                    );
                }
            }
            "named_imports" => {
                // Recurse into `{ a, b }`
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "import_specifier" {
                        let (sym_name, original_name) =
                            extract_specifier_names(content, inner_child);
                        if !sym_name.is_empty() {
                            add_import_symbol(
                                result,
                                file_stem,
                                &sym_name,
                                original_name.as_deref(),
                                source_module,
                                path,
                                inner_child,
                            );
                        }
                    }
                }
            }
            "namespace_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(content, name_node).to_string();
                    if !name.is_empty() {
                        add_import_symbol(
                            result,
                            file_stem,
                            &name,
                            None,
                            source_module,
                            path,
                            child,
                        );
                    }
                }
            }
            _ => {}
        }
    }
}

/// Helper to add an import reference + symbol pair.
///
/// Creates both a `Reference` and a `Symbol` for each imported name. When
/// `source_module` is provided, adds `"from:<module>"` annotation so that
/// `resolve_cross_file_imports()` can match the import to its target file.
/// When `original_name` differs from `name`, adds `"original_name:<name>"`
/// so aliased imports resolve to the correct definition.
fn add_import_symbol(
    result: &mut ParseResult,
    file_stem: &str,
    name: &str,
    original_name: Option<&str>,
    source_module: Option<&str>,
    path: &Path,
    node: Node,
) {
    result.references.push(Reference {
        id: ReferenceId::default(),
        from: SymbolId::default(),
        to: SymbolId::default(),
        kind: ReferenceKind::Import,
        location: node_location(path, node),
        resolution: ResolutionStatus::Partial("external".to_string()),
    });

    let mut annotations = vec!["import".to_string()];
    if let Some(src) = source_module {
        if !src.is_empty() {
            annotations.push(format!("from:{}", src));
        }
    }
    if let Some(orig) = original_name {
        if orig != name {
            annotations.push(format!("original_name:{}", orig));
        }
    }

    result.symbols.push(Symbol {
        id: SymbolId::default(),
        kind: SymbolKind::Module,
        name: name.to_string(),
        qualified_name: format!("{}::import::{}", file_stem, name),
        visibility: Visibility::Private,
        signature: None,
        location: node_location(path, node),
        resolution: ResolutionStatus::Partial("import".to_string()),
        scope: ScopeId::default(),
        annotations,
    });
}

/// Recursively walks the AST to extract all function/method call references.
///
/// Creates a `Reference` with `kind: ReferenceKind::Call` for each `call_expression`
/// node found. The callee name is stored in
/// `resolution: ResolutionStatus::Partial("call:<name>")` for later resolution
/// by `populate_graph`.
///
/// CJS `require()` calls with a string literal argument are intercepted and
/// converted to import symbols instead of regular call references.
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
        if let Some(func_node) = node.child_by_field_name("function") {
            if let Some(name) = extract_callee_name(content, func_node) {
                // Intercept require() calls with string literal argument
                if name == "require" {
                    if let Some(source) = extract_require_source(content, node) {
                        let file_stem = path
                            .file_name()
                            .map(|s| s.to_string_lossy().to_string())
                            .unwrap_or_else(|| "unknown".to_string());

                        // Check for destructured require: const { x, y } = require(...)
                        let destructured_bindings =
                            extract_destructured_require_bindings(content, node);

                        if !destructured_bindings.is_empty() {
                            // Create individual import symbols per binding
                            for (local_name, original_name) in &destructured_bindings {
                                let mut annotations = vec![
                                    "import".to_string(),
                                    format!("from:{}", source),
                                    "cjs".to_string(),
                                ];
                                if let Some(orig) = original_name {
                                    annotations.push(format!("original_name:{}", orig));
                                }

                                result.references.push(Reference {
                                    id: ReferenceId::default(),
                                    from: SymbolId::default(),
                                    to: SymbolId::default(),
                                    kind: ReferenceKind::Import,
                                    location: node_location(path, node),
                                    resolution: ResolutionStatus::Partial("external".to_string()),
                                });

                                result.symbols.push(Symbol {
                                    id: SymbolId::default(),
                                    kind: SymbolKind::Variable,
                                    name: local_name.clone(),
                                    qualified_name: format!(
                                        "{}::import::{}",
                                        file_stem, local_name
                                    ),
                                    visibility: Visibility::Private,
                                    signature: None,
                                    location: node_location(path, node),
                                    resolution: ResolutionStatus::Partial("import".to_string()),
                                    scope: ScopeId::default(),
                                    annotations,
                                });
                            }
                        } else {
                            // Whole-module require: existing behavior
                            let var_name = extract_require_var_name(content, node);
                            let import_name = var_name.as_deref().unwrap_or(&name);

                            let mut annotations = vec![
                                "import".to_string(),
                                format!("from:{}", source),
                                "cjs".to_string(),
                            ];
                            if var_name.is_none() {
                                annotations.push("require".to_string());
                            }

                            result.references.push(Reference {
                                id: ReferenceId::default(),
                                from: SymbolId::default(),
                                to: SymbolId::default(),
                                kind: ReferenceKind::Import,
                                location: node_location(path, node),
                                resolution: ResolutionStatus::Partial("external".to_string()),
                            });

                            result.symbols.push(Symbol {
                                id: SymbolId::default(),
                                kind: SymbolKind::Module,
                                name: import_name.to_string(),
                                qualified_name: format!("{}::import::{}", file_stem, import_name),
                                visibility: Visibility::Private,
                                signature: None,
                                location: node_location(path, node),
                                resolution: ResolutionStatus::Partial("import".to_string()),
                                scope: ScopeId::default(),
                                annotations,
                            });
                        }

                        // Don't emit a regular call reference for require()
                        // Still recurse into children
                        let mut cursor = node.walk();
                        for child in node.children(&mut cursor) {
                            extract_all_calls(result, content, path, child, depth + 1);
                        }
                        return;
                    }
                }

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

/// Extracts the string literal argument from a `require()` call expression.
///
/// For `require('./bar')`, returns `Some("./bar")`. Returns `None` if the
/// argument is not a string literal (e.g., `require(variable)`).
fn extract_require_source(content: &[u8], call_node: Node) -> Option<String> {
    let args = call_node.child_by_field_name("arguments")?;
    let mut cursor = args.walk();
    for child in args.children(&mut cursor) {
        if child.kind() == "string" {
            let raw = node_text(content, child);
            let trimmed = raw
                .trim_start_matches(['\'', '"'])
                .trim_end_matches(['\'', '"']);
            return Some(trimmed.to_string());
        }
    }
    None
}

/// Extracts the variable name that a `require()` call is assigned to.
///
/// For `const mod = require('./bar')`, walks up from the call_expression to find
/// the enclosing `variable_declarator` and extracts its `name` field.
fn extract_require_var_name(content: &[u8], call_node: Node) -> Option<String> {
    // Walk up to find the variable_declarator parent
    let parent = call_node.parent()?;
    if parent.kind() == "variable_declarator" {
        let name_node = parent.child_by_field_name("name")?;
        // Skip object_pattern (destructured requires) and array_pattern
        if name_node.kind() == "object_pattern" || name_node.kind() == "array_pattern" {
            return None;
        }
        let name = node_text(content, name_node).to_string();
        if !name.is_empty() {
            return Some(name);
        }
    }
    None
}

/// Extracts individual bindings from a destructured `require()` call.
///
/// For `const { x, y: alias } = require('./utils')`, returns
/// `[("x", None), ("alias", Some("y"))]`. Returns an empty vec if the
/// require is not destructured (e.g., `const mod = require('./utils')`).
fn extract_destructured_require_bindings(
    content: &[u8],
    call_node: Node,
) -> Vec<(String, Option<String>)> {
    let parent = match call_node.parent() {
        Some(p) if p.kind() == "variable_declarator" => p,
        _ => return Vec::new(),
    };

    let name_node = match parent.child_by_field_name("name") {
        Some(n) if n.kind() == "object_pattern" => n,
        _ => return Vec::new(),
    };

    let mut bindings = Vec::new();
    let mut cursor = name_node.walk();
    for child in name_node.children(&mut cursor) {
        match child.kind() {
            "shorthand_property_identifier_pattern" => {
                let binding_name = node_text(content, child).to_string();
                if !binding_name.is_empty() {
                    bindings.push((binding_name, None));
                }
            }
            "pair_pattern" => {
                let key = child
                    .child_by_field_name("key")
                    .map(|k| node_text(content, k).to_string());
                let value = child
                    .child_by_field_name("value")
                    .map(|v| node_text(content, v).to_string());
                if let Some(alias) = value {
                    if !alias.is_empty() {
                        bindings.push((alias, key));
                    }
                }
            }
            _ => {}
        }
    }
    bindings
}

/// Extracts the callee name from a call expression's `function` field.
///
/// Returns `Some(name)` for identifiers (`func`) and member expressions
/// (`obj.method`). Returns `None` for complex expressions.
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
        "member_expression" => {
            let object = func_node.child_by_field_name("object")?;
            let property = func_node.child_by_field_name("property")?;
            let obj_name = node_text(content, object);
            let prop_name = node_text(content, property);
            if prop_name.is_empty() {
                None
            } else {
                Some(format!("{}.{}", obj_name, prop_name))
            }
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// TypeScript preprocessing
// ---------------------------------------------------------------------------

/// Collapses runs of 2+ whitespace characters into a single space.
/// Applied to function/method signatures extracted from preprocessed TS content,
/// where `strip_type_annotations()` leaves multi-space gaps.
fn collapse_signature_whitespace(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_space = false;
    for ch in s.chars() {
        if ch == ' ' || ch == '\t' {
            if !prev_was_space {
                result.push(' ');
            }
            prev_was_space = true;
        } else {
            prev_was_space = false;
            result.push(ch);
        }
    }
    result
}

fn is_typescript_file(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| matches!(e, "ts" | "tsx"))
        .unwrap_or(false)
}

fn preprocess_typescript(content: &str) -> String {
    let mut result = String::with_capacity(content.len());
    let lines: Vec<&str> = content.split('\n').collect();
    let mut in_ts_block = false;
    let mut brace_depth: i32 = 0;
    let mut in_type_alias = false;

    for (idx, line) in lines.iter().enumerate() {
        if idx > 0 {
            result.push('\n');
        }
        if in_type_alias {
            blank_line(&mut result, line);
            if line.contains(';') {
                in_type_alias = false;
            }
            continue;
        }
        if in_ts_block {
            let (d, ended) = count_braces_in_line(line, brace_depth);
            brace_depth = d;
            blank_line(&mut result, line);
            if ended {
                in_ts_block = false;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            result.push_str(line);
            continue;
        }
        if let Some(rest) = detect_ts_block_start(trimmed) {
            if let Some(open_idx) = rest.find('{') {
                let (d, ended) = count_braces_in_line(&rest[open_idx..], 0);
                blank_line(&mut result, line);
                if !ended {
                    in_ts_block = true;
                    brace_depth = d;
                }
            } else if rest.contains(';') {
                blank_line(&mut result, line);
            } else {
                blank_line(&mut result, line);
                in_type_alias = true;
            }
            continue;
        }
        result.push_str(&strip_ts_line_syntax(line));
    }
    result
}

fn blank_line(result: &mut String, line: &str) {
    for c in line.chars() {
        result.push(if c == '\t' { '\t' } else { ' ' });
    }
}

fn detect_ts_block_start(trimmed: &str) -> Option<&str> {
    let mut s = trimmed;
    if let Some(r) = s.strip_prefix("export") {
        s = r.trim_start();
    }
    if let Some(r) = s.strip_prefix("declare") {
        s = r.trim_start();
    }
    if let Some(r) = s.strip_prefix("const") {
        let rt = r.trim_start();
        if rt.starts_with("enum") {
            s = rt;
        }
    }
    if s.starts_with("interface ") || s.starts_with("interface\t") {
        let a = s["interface".len()..].trim_start();
        return a.find(['{', ';']).map(|i| &a[i..]).or(Some(a));
    }
    if s.starts_with("enum ") || s.starts_with("enum\t") {
        let a = s["enum".len()..].trim_start();
        return a.find('{').map(|i| &a[i..]).or(Some(a));
    }
    if s.starts_with("type ") || s.starts_with("type\t") {
        let a = s["type".len()..].trim_start();
        if a.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            return a.find('=').map(|i| &a[i..]).or(Some(a));
        }
    }
    None
}

fn count_braces_in_line(line: &str, mut depth: i32) -> (i32, bool) {
    let mut in_str = false;
    let mut sc = ' ';
    let mut prev = ' ';
    for ch in line.chars() {
        if in_str {
            if ch == sc && prev != '\\' {
                in_str = false;
            }
        } else {
            match ch {
                '"' | '\'' | '`' => {
                    in_str = true;
                    sc = ch;
                }
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth <= 0 {
                        return (0, true);
                    }
                }
                _ => {}
            }
        }
        prev = ch;
    }
    (depth, false)
}

fn strip_ts_line_syntax(line: &str) -> String {
    let mut s = line.to_string();
    s = strip_keyword_after(&s, "import", "type");
    s = strip_keyword_after(&s, "export", "type");
    s = strip_leading_keyword(&s, "declare");
    s = strip_leading_keyword(&s, "abstract");
    for m in &["private", "public", "protected", "readonly"] {
        s = strip_leading_modifier(&s, m);
    }
    s = strip_generics(&s);
    s = strip_type_annotations(&s);
    s
}

fn strip_keyword_after(line: &str, kw1: &str, kw2: &str) -> String {
    if let Some(p) = line.find(kw1) {
        let after = p + kw1.len();
        let rest = &line[after..];
        let ws = rest.len() - rest.trim_start().len();
        let aw = &rest[ws..];
        if aw.starts_with(kw2) {
            let end = after + ws + kw2.len();
            let tail = &line[end..];
            if tail.is_empty() || tail.starts_with(|c: char| c.is_whitespace() || c == '{') {
                let mut r = String::with_capacity(line.len());
                r.push_str(&line[..after + ws]);
                r.extend(std::iter::repeat_n(' ', kw2.len()));
                r.push_str(tail);
                return r;
            }
        }
    }
    line.to_string()
}

fn strip_leading_keyword(line: &str, keyword: &str) -> String {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if let Some(rest) = trimmed.strip_prefix("export") {
        let rt = rest.trim_start();
        if let Some(after) = rt.strip_prefix(keyword) {
            if after.is_empty() || after.starts_with(|c: char| c.is_whitespace()) {
                let ks = indent + "export".len() + (rest.len() - rt.len());
                let ke = ks + keyword.len();
                let mut r = String::with_capacity(line.len());
                r.push_str(&line[..ks]);
                r.extend(std::iter::repeat_n(' ', keyword.len()));
                r.push_str(&line[ke..]);
                return r;
            }
        }
    }
    if let Some(after) = trimmed.strip_prefix(keyword) {
        if after.is_empty() || after.starts_with(|c: char| c.is_whitespace()) {
            let ke = indent + keyword.len();
            let mut r = String::with_capacity(line.len());
            r.push_str(&line[..indent]);
            r.extend(std::iter::repeat_n(' ', keyword.len()));
            r.push_str(&line[ke..]);
            return r;
        }
    }
    line.to_string()
}

fn strip_leading_modifier(line: &str, keyword: &str) -> String {
    let trimmed = line.trim_start();
    let indent = line.len() - trimmed.len();
    if let Some(after) = trimmed.strip_prefix(keyword) {
        if after.starts_with(|c: char| c.is_whitespace()) {
            let ke = indent + keyword.len();
            let mut r = String::with_capacity(line.len());
            r.push_str(&line[..indent]);
            r.extend(std::iter::repeat_n(' ', keyword.len()));
            r.push_str(&line[ke..]);
            return r;
        }
    }
    line.to_string()
}

fn strip_generics(line: &str) -> String {
    let b = line.as_bytes();
    let len = b.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    while i < len {
        if b[i] == b'<' && is_generic_open(b, i) {
            let start = i;
            let mut depth = 1;
            i += 1;
            while i < len && depth > 0 {
                match b[i] {
                    b'<' => depth += 1,
                    b'>' => depth -= 1,
                    b'\'' | b'"' | b'`' => {
                        let q = b[i];
                        i += 1;
                        while i < len && b[i] != q {
                            i += 1;
                        }
                        if i < len {
                            i += 1;
                        }
                        continue;
                    }
                    _ => {}
                }
                i += 1;
            }
            out.extend(std::iter::repeat_n(b' ', i - start));
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| line.to_string())
}

fn is_generic_open(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let mut j = i - 1;
    while j > 0 && bytes[j] == b' ' {
        j -= 1;
    }
    let p = bytes[j];
    p.is_ascii_alphanumeric() || p == b'_' || p == b'$'
}

fn strip_type_annotations(line: &str) -> String {
    let bytes = line.as_bytes();
    let len = bytes.len();
    let mut out = Vec::with_capacity(len);
    let mut i = 0;
    let mut pd: i32 = 0;
    let mut in_str = false;
    let mut sc = b' ';
    while i < len {
        let c = bytes[i];
        if in_str {
            out.push(c);
            if c == sc && (i == 0 || bytes[i - 1] != b'\\') {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                in_str = true;
                sc = c;
                out.push(c);
                i += 1;
            }
            b'(' => {
                pd += 1;
                out.push(c);
                i += 1;
            }
            b')' => {
                pd -= 1;
                out.push(c);
                i += 1;
                if pd <= 0 {
                    if let Some(co) = find_type_colon(&bytes[i..]) {
                        let cp = i + co;
                        let ep = find_annotation_end(&bytes[cp..], false);
                        let se = cp + ep;
                        out.extend(std::iter::repeat_n(b' ', se - i));
                        i = se;
                    }
                }
            }
            b':' if pd > 0 => {
                if is_param_type_colon(bytes, i) {
                    let ep = find_annotation_end(&bytes[i..], true);
                    out.extend(std::iter::repeat_n(b' ', ep));
                    i += ep;
                } else {
                    out.push(c);
                    i += 1;
                }
            }
            _ => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8(out).unwrap_or_else(|_| line.to_string())
}

fn find_type_colon(bytes: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < bytes.len() && bytes[i] == b' ' {
        i += 1;
    }
    if i < bytes.len() && bytes[i] == b':' {
        if i + 1 < bytes.len() && bytes[i + 1] == b':' {
            return None;
        }
        return Some(i);
    }
    None
}

fn is_param_type_colon(bytes: &[u8], i: usize) -> bool {
    if i == 0 {
        return false;
    }
    let p = bytes[i - 1];
    p.is_ascii_alphanumeric() || p == b'_' || p == b'$' || p == b'?'
}

fn find_annotation_end(bytes: &[u8], in_params: bool) -> usize {
    let mut i = 1;
    let len = bytes.len();
    let mut ad: i32 = 0;
    let mut pd: i32 = 0;
    let mut in_str = false;
    let mut sc = b' ';
    while i < len {
        let c = bytes[i];
        if in_str {
            if c == sc && (i == 0 || bytes[i - 1] != b'\\') {
                in_str = false;
            }
            i += 1;
            continue;
        }
        match c {
            b'\'' | b'"' | b'`' => {
                in_str = true;
                sc = c;
                i += 1;
            }
            b'<' => {
                ad += 1;
                i += 1;
            }
            b'>' => {
                if ad > 0 {
                    ad -= 1;
                }
                i += 1;
            }
            b'(' => {
                pd += 1;
                i += 1;
            }
            b')' if pd > 0 => {
                pd -= 1;
                i += 1;
            }
            _ if ad > 0 || pd > 0 => i += 1,
            b',' | b')' if in_params => return i,
            b'=' if in_params => return i,
            b'{' if !in_params => return i,
            b'=' if !in_params && i + 1 < len && bytes[i + 1] == b'>' => return i,
            b';' if !in_params => return i,
            _ => i += 1,
        }
    }
    len
}

fn pre_extract_ts_entities(content: &str, path: &Path, file_stem: &str) -> Vec<Symbol> {
    let mut entities = Vec::new();
    let lines: Vec<&str> = content.split('\n').collect();
    let mut in_block = false;
    let mut brace_depth: i32 = 0;
    for (li, line) in lines.iter().enumerate() {
        if in_block {
            let (d, ended) = count_braces_in_line(line, brace_depth);
            brace_depth = d;
            if ended {
                in_block = false;
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.starts_with("//") || trimmed.starts_with("/*") || trimmed.starts_with('*') {
            continue;
        }
        let ln = (li + 1) as u32;
        if let Some(ent) = try_extract_ts_entity(trimmed, path, file_stem, ln) {
            entities.push(ent);
            if let Some(bp) = trimmed.find('{') {
                let (d, ended) = count_braces_in_line(&trimmed[bp..], 0);
                if !ended {
                    in_block = true;
                    brace_depth = d;
                }
            }
        }
    }
    entities
}

fn try_extract_ts_entity(
    trimmed: &str,
    path: &Path,
    file_stem: &str,
    line_num: u32,
) -> Option<Symbol> {
    let mut s = trimmed;
    let mut exported = false;
    let mut is_declare = false;
    if let Some(r) = s.strip_prefix("export") {
        exported = true;
        s = r.trim_start();
    }
    if let Some(r) = s.strip_prefix("declare") {
        is_declare = true;
        s = r.trim_start();
    }
    if let Some(r) = s.strip_prefix("const") {
        let rt = r.trim_start();
        if rt.starts_with("enum") {
            s = rt;
        }
    }
    let vis = if exported {
        Visibility::Public
    } else {
        Visibility::Private
    };
    let loc = Location {
        file: path.to_path_buf(),
        line: line_num,
        column: 1,
        end_line: line_num,
        end_column: 1,
    };
    // Only pre-extract function/class for `declare` variants that are bodyless
    // (no `{`). Bodyless forms like `declare function greet(): void;` become
    // `function greet();` after stripping, which tree-sitter-javascript cannot
    // parse. Forms WITH bodies survive preprocessing and tree-sitter handles them,
    // so pre-extracting those would create duplicates.
    if is_declare
        && !trimmed.contains('{')
        && (s.starts_with("function ") || s.starts_with("function\t"))
    {
        let name = extract_identifier(s["function".len()..].trim_start())?;
        return Some(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Function,
            name: name.clone(),
            qualified_name: format!("{}::{}", file_stem, name),
            visibility: vis,
            signature: None,
            location: loc,
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec!["declare".to_string()],
        });
    }
    if is_declare && !trimmed.contains('{') && (s.starts_with("class ") || s.starts_with("class\t"))
    {
        let name = extract_identifier(s["class".len()..].trim_start())?;
        return Some(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Class,
            name: name.clone(),
            qualified_name: format!("{}::{}", file_stem, name),
            visibility: vis,
            signature: None,
            location: loc,
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec!["declare".to_string()],
        });
    }
    if s.starts_with("interface ") || s.starts_with("interface\t") {
        let name = extract_identifier(s["interface".len()..].trim_start())?;
        return Some(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Interface,
            name: name.clone(),
            qualified_name: format!("{}::{}", file_stem, name),
            visibility: vis,
            signature: None,
            location: loc,
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec![],
        });
    }
    if s.starts_with("enum ") || s.starts_with("enum\t") {
        let name = extract_identifier(s["enum".len()..].trim_start())?;
        return Some(Symbol {
            id: SymbolId::default(),
            kind: SymbolKind::Enum,
            name: name.clone(),
            qualified_name: format!("{}::{}", file_stem, name),
            visibility: vis,
            signature: None,
            location: loc,
            resolution: ResolutionStatus::Resolved,
            scope: ScopeId::default(),
            annotations: vec![],
        });
    }
    if s.starts_with("type ") || s.starts_with("type\t") {
        let after = s["type".len()..].trim_start();
        if after.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_') {
            let name = extract_identifier(after)?;
            if name == "of" {
                return None;
            }
            return Some(Symbol {
                id: SymbolId::default(),
                kind: SymbolKind::Interface,
                name: name.clone(),
                qualified_name: format!("{}::{}", file_stem, name),
                visibility: vis,
                signature: None,
                location: loc,
                resolution: ResolutionStatus::Resolved,
                scope: ScopeId::default(),
                annotations: vec!["type_alias".to_string()],
            });
        }
    }
    None
}

fn extract_identifier(s: &str) -> Option<String> {
    let name: String = s
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
        .collect();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn parse_js(filename: &str, content: &str) -> ParseResult {
        let adapter = JsAdapter::new();
        let path = PathBuf::from(filename);
        adapter
            .parse_file(&path, content)
            .unwrap_or_else(|e| panic!("Failed to parse {}: {:?}", filename, e))
    }

    // -----------------------------------------------------------------------
    // P0 — can_handle() contract
    // -----------------------------------------------------------------------

    #[test]
    fn test_can_handle_js() {
        let adapter = JsAdapter::new();
        assert!(adapter.can_handle(Path::new("foo.js")));
    }

    #[test]
    fn test_can_handle_jsx() {
        let adapter = JsAdapter::new();
        assert!(adapter.can_handle(Path::new("component.jsx")));
    }

    #[test]
    fn test_can_handle_ts() {
        let adapter = JsAdapter::new();
        assert!(adapter.can_handle(Path::new("service.ts")));
    }

    #[test]
    fn test_can_handle_tsx() {
        let adapter = JsAdapter::new();
        assert!(adapter.can_handle(Path::new("app.tsx")));
    }

    #[test]
    fn test_cannot_handle_py() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("main.py")));
    }

    #[test]
    fn test_cannot_handle_rs() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("lib.rs")));
    }

    #[test]
    fn test_cannot_handle_json() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("package.json")));
    }

    #[test]
    fn test_cannot_handle_no_extension() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("Makefile")));
    }

    #[test]
    fn test_can_handle_dot_mjs() {
        // .mjs is ES module format — accepted
        let adapter = JsAdapter::new();
        assert!(adapter.can_handle(Path::new("module.mjs")));
    }

    #[test]
    fn test_can_handle_case_sensitivity() {
        let adapter = JsAdapter::new();
        // Linux is case-sensitive. .JS != .js
        assert!(!adapter.can_handle(Path::new("FOO.JS")));
    }

    #[test]
    fn test_cannot_handle_js_in_directory_name() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("js-utils/readme.md")));
    }

    #[test]
    fn test_cannot_handle_backup_file() {
        let adapter = JsAdapter::new();
        assert!(!adapter.can_handle(Path::new("old.js.bak")));
    }

    // -----------------------------------------------------------------------
    // P0 — language_name()
    // -----------------------------------------------------------------------

    #[test]
    fn test_language_name_is_javascript() {
        let adapter = JsAdapter::new();
        assert_eq!(adapter.language_name(), "javascript");
    }

    // -----------------------------------------------------------------------
    // P0 — Named function extraction (hard gate)
    // -----------------------------------------------------------------------

    #[test]
    fn test_named_function_extraction() {
        let result = parse_js(
            "named.js",
            "function greet(name) {\n  return 'hello ' + name;\n}\n",
        );
        let functions: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            !functions.is_empty(),
            "Must extract at least 1 function, got {}",
            functions.len()
        );
        let greet = functions
            .iter()
            .find(|s| s.name == "greet")
            .expect("Must find function named 'greet'");
        assert_eq!(greet.kind, SymbolKind::Function);
        assert!(greet.location.line >= 1, "Location must be 1-based");
    }

    // -----------------------------------------------------------------------
    // P0 — Arrow function extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_arrow_function_const() {
        let result = parse_js("arrow.js", "const add = (a, b) => a + b;\n");
        let add = result
            .symbols
            .iter()
            .find(|s| s.name == "add")
            .expect("Must extract arrow function assigned to 'add'");
        assert_eq!(add.kind, SymbolKind::Function);
    }

    #[test]
    fn test_arrow_function_let() {
        let result = parse_js(
            "arrow_let.js",
            "let multiply = (a, b) => { return a * b; };\n",
        );
        let mult = result
            .symbols
            .iter()
            .find(|s| s.name == "multiply")
            .expect("Must extract arrow function assigned via 'let'");
        assert_eq!(mult.kind, SymbolKind::Function);
    }

    #[test]
    fn test_arrow_function_var() {
        let result = parse_js("arrow_var.js", "var divide = (a, b) => a / b;\n");
        let div = result
            .symbols
            .iter()
            .find(|s| s.name == "divide")
            .expect("Must extract arrow function assigned via 'var'");
        assert_eq!(div.kind, SymbolKind::Function);
    }

    // -----------------------------------------------------------------------
    // P0 — Method definition (inside class)
    // -----------------------------------------------------------------------

    #[test]
    fn test_method_in_class() {
        let content = r#"
class Calculator {
    add(a, b) {
        return a + b;
    }
    subtract(a, b) {
        return a - b;
    }
}
"#;
        let result = parse_js("methods.js", content);
        let methods: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .collect();
        assert!(
            methods.len() >= 2,
            "Must extract at least 2 methods, got {}",
            methods.len()
        );
        assert!(
            methods.iter().any(|m| m.name == "add"),
            "Must find 'add' method"
        );
        assert!(
            methods.iter().any(|m| m.name == "subtract"),
            "Must find 'subtract' method"
        );
    }

    // -----------------------------------------------------------------------
    // P1 — Adversarial tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_js_file_no_panic() {
        let result = parse_js("empty.js", "");
        assert_eq!(result.symbols.len(), 0, "Empty file must produce 0 symbols");
        assert!(result.scopes.len() >= 1, "Must have at least file scope");
    }

    #[test]
    fn test_comments_only_js_no_symbols() {
        let content = "// This is a comment\n/* Block comment */\n/** JSDoc */\n";
        let result = parse_js("comments.js", content);
        assert_eq!(
            result.symbols.len(),
            0,
            "Comments-only file must produce 0 symbols"
        );
    }

    #[test]
    fn test_syntax_error_no_panic() {
        let content = "function broken( { return; }\nfunction valid() { return 1; }\n";
        let _result = parse_js("syntax_error.js", content);
        // Must not panic. Error result is acceptable.
    }

    #[test]
    fn test_partial_file_no_panic() {
        let content = "function incomplete(";
        let _result = parse_js("partial.js", content);
        // Truncated file — tree-sitter handles gracefully.
    }

    #[test]
    fn test_js_with_error_node_extracts_valid_symbols() {
        let content = "function good() { return 1; }\n{{{invalid\nfunction alsogood() {}\n";
        let result = parse_js("mixed_errors.js", content);
        let fns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert!(
            !fns.is_empty(),
            "Should recover at least 1 function from mixed valid/invalid JS"
        );
    }

    #[test]
    fn test_deeply_nested_functions() {
        let content = r#"
function level1() {
    function level2() {
        function level3() {
            function level4() {
                function level5() {
                    return 42;
                }
            }
        }
    }
}
"#;
        let result = parse_js("nested.js", content);
        let fns: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(
            fns.len(),
            5,
            "Must extract all 5 nested functions, got {}",
            fns.len()
        );
        assert!(
            fns.iter().any(|f| f.name == "level5"),
            "Must find innermost function"
        );
    }

    #[test]
    fn test_unicode_function_name() {
        let content = "function café() { return 'coffee'; }\n";
        let result = parse_js("unicode.js", content);
        let cafe = result
            .symbols
            .iter()
            .find(|s| s.name == "café")
            .expect("Must extract function with unicode name 'café'");
        assert_eq!(cafe.kind, SymbolKind::Function);
    }

    #[test]
    fn test_unicode_arrow_function() {
        let content = "const données = () => {};\n";
        let result = parse_js("unicode_arrow.js", content);
        let d = result
            .symbols
            .iter()
            .find(|s| s.name == "données")
            .expect("Must extract arrow function with unicode name");
        assert_eq!(d.kind, SymbolKind::Function);
    }

    #[test]
    fn test_iife_no_panic() {
        let content = "(function() { console.log('init'); })();\n";
        let _result = parse_js("iife.js", content);
        // IIFE is anonymous — no name to extract. Must not panic.
    }

    #[test]
    fn test_arrow_iife_no_panic() {
        let content = "(() => { console.log('init'); })();\n";
        let _result = parse_js("arrow_iife.js", content);
        // Same as above — anonymous, no crash.
    }

    #[test]
    fn test_async_function() {
        let content = "async function fetchData() { return await fetch('/api'); }\n";
        let result = parse_js("async.js", content);
        let fetch_fn = result
            .symbols
            .iter()
            .find(|s| s.name == "fetchData")
            .expect("Must extract async function 'fetchData'");
        assert_eq!(fetch_fn.kind, SymbolKind::Function);
    }

    #[test]
    fn test_generator_function() {
        let content = "function* range(n) { for(let i=0; i<n; i++) yield i; }\n";
        let result = parse_js("generator.js", content);
        let gen = result
            .symbols
            .iter()
            .find(|s| s.name == "range")
            .expect("Must extract generator function 'range'");
        assert_eq!(gen.kind, SymbolKind::Function);
    }

    #[test]
    fn test_async_arrow_function() {
        let content = "const fetchUser = async (id) => { return await db.get(id); };\n";
        let result = parse_js("async_arrow.js", content);
        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "fetchUser")
            .expect("Must extract async arrow function 'fetchUser'");
        assert_eq!(f.kind, SymbolKind::Function);
    }

    // -----------------------------------------------------------------------
    // P1 — Export visibility
    // -----------------------------------------------------------------------

    #[test]
    fn test_exported_function_is_public() {
        let content = "export function publicFn() {}\n";
        let result = parse_js("export.js", content);
        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "publicFn")
            .expect("Must extract exported function");
        assert_eq!(f.visibility, Visibility::Public);
    }

    #[test]
    fn test_non_exported_function_is_private() {
        let content = "function privateFn() {}\n";
        let result = parse_js("private.js", content);
        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "privateFn")
            .expect("Must extract non-exported function");
        assert_eq!(f.visibility, Visibility::Private);
    }

    #[test]
    fn test_export_default_function() {
        let content = "export default function handler() {}\n";
        let result = parse_js("export_default.js", content);
        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "handler")
            .expect("Must extract default-exported function");
        assert_eq!(f.visibility, Visibility::Public);
    }

    #[test]
    fn test_exported_arrow_is_public() {
        let content = "export const process = (data) => data.map(x => x * 2);\n";
        let result = parse_js("export_arrow.js", content);
        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "process")
            .expect("Must extract exported arrow function");
        assert_eq!(f.visibility, Visibility::Public);
    }

    #[test]
    fn test_multiple_exports_all_public() {
        let content = r#"
export function a() {}
export function b() {}
function c() {}
"#;
        let result = parse_js("multi_export.js", content);
        let a = result.symbols.iter().find(|s| s.name == "a").unwrap();
        let b = result.symbols.iter().find(|s| s.name == "b").unwrap();
        let c = result.symbols.iter().find(|s| s.name == "c").unwrap();
        assert_eq!(a.visibility, Visibility::Public);
        assert_eq!(b.visibility, Visibility::Public);
        assert_eq!(c.visibility, Visibility::Private);
    }

    // -----------------------------------------------------------------------
    // P1 — Scope structure
    // -----------------------------------------------------------------------

    #[test]
    fn test_file_scope_always_exists() {
        let content = "function f() {}\n";
        let result = parse_js("scope.js", content);
        assert!(
            result.scopes.iter().any(|s| s.kind == ScopeKind::File),
            "Must always have a File scope"
        );
    }

    #[test]
    fn test_function_creates_scope() {
        let content = "function outer() {\n  function inner() {}\n}\n";
        let result = parse_js("scopes.js", content);
        let fn_scopes: Vec<_> = result
            .scopes
            .iter()
            .filter(|s| s.kind == ScopeKind::Function)
            .collect();
        assert!(
            fn_scopes.len() >= 2,
            "Both outer and inner should create Function scopes"
        );
    }

    // -----------------------------------------------------------------------
    // P1 — Location accuracy
    // -----------------------------------------------------------------------

    #[test]
    fn test_function_location_line_number() {
        let content = "// comment\n// another\nfunction target() {}\n";
        let result = parse_js("location.js", content);
        let target = result.symbols.iter().find(|s| s.name == "target").unwrap();
        assert_eq!(
            target.location.line, 3,
            "Function on line 3 must report line 3, got {}",
            target.location.line
        );
    }

    #[test]
    fn test_arrow_function_location() {
        let content = "\n\nconst fn_x = () => {};\n";
        let result = parse_js("arrow_loc.js", content);
        let f = result.symbols.iter().find(|s| s.name == "fn_x").unwrap();
        // Arrow function location should point to the const declaration, line 3
        assert!(
            f.location.line >= 3,
            "Arrow on line 3 should report >= 3, got {}",
            f.location.line
        );
    }

    // -----------------------------------------------------------------------
    // P1 — Qualified names
    // -----------------------------------------------------------------------

    #[test]
    fn test_qualified_name_top_level_function() {
        let result = parse_js("qname.js", "function greet() {}\n");
        let greet = result.symbols.iter().find(|s| s.name == "greet").unwrap();
        assert!(
            greet.qualified_name.contains("greet"),
            "Qualified name must contain 'greet', got '{}'",
            greet.qualified_name
        );
    }

    #[test]
    fn test_qualified_name_method_includes_class() {
        let content = "class Dog {\n  bark() {}\n}\n";
        let result = parse_js("qname_method.js", content);
        let bark = result.symbols.iter().find(|s| s.name == "bark").unwrap();
        assert!(
            bark.qualified_name.contains("Dog"),
            "Method qualified name must include class 'Dog', got '{}'",
            bark.qualified_name
        );
        assert!(
            bark.qualified_name.contains("bark"),
            "Method qualified name must include method 'bark', got '{}'",
            bark.qualified_name
        );
    }

    // -----------------------------------------------------------------------
    // P1 — Binary / null content safety
    // -----------------------------------------------------------------------

    #[test]
    fn test_binary_content_no_crash() {
        let adapter = JsAdapter::new();
        let binary = String::from_utf8_lossy(b"\x00\x01\x02\x03\xff\xfe\xfd").to_string();
        let _result = adapter.parse_file(Path::new("binary.js"), &binary);
        // Must not panic. Error result is acceptable.
    }

    #[test]
    fn test_null_bytes_in_source() {
        let adapter = JsAdapter::new();
        let content = "function good() {}\n\x00\nfunction also() {}";
        let _result = adapter.parse_file(Path::new("nulls.js"), content);
        // Must not panic.
    }

    #[test]
    fn test_extremely_long_line_no_crash() {
        let adapter = JsAdapter::new();
        let content = format!("const x = \"{}\";\n", "a".repeat(100_000));
        assert!(adapter.parse_file(Path::new("long.js"), &content).is_ok());
    }

    // -----------------------------------------------------------------------
    // P2 — Stretch: Class extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_class_extraction() {
        let content =
            "class Animal {\n  constructor(name) { this.name = name; }\n  speak() {}\n}\n";
        let result = parse_js("class.js", content);
        let cls = result.symbols.iter().find(|s| s.kind == SymbolKind::Class);
        let animal = cls.expect("Must extract class 'Animal'");
        assert_eq!(animal.name, "Animal");
    }

    // -----------------------------------------------------------------------
    // P2 — Stretch: Import extraction
    // -----------------------------------------------------------------------

    #[test]
    fn test_import_creates_reference() {
        let content = "import { readFile } from 'fs';\n";
        let result = parse_js("import.js", content);
        let imports: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(!imports.is_empty(), "Must create import reference");
    }

    // -----------------------------------------------------------------------
    // Cycle 5 — Export { name } clause (D3)
    // -----------------------------------------------------------------------

    /// Core fix: `function foo() {} export { foo }` — foo must be Public.
    #[test]
    fn test_export_clause_named_function_becomes_public() {
        let content = "function foo() { return 1; }\n\nexport { foo };\n";
        let result = parse_js("export_clause.js", content);

        let foo = result
            .symbols
            .iter()
            .find(|s| s.name == "foo")
            .expect("Must find function 'foo'");
        assert_eq!(
            foo.visibility,
            Visibility::Public,
            "export {{ foo }} must set foo's visibility to Public. Got: {:?}",
            foo.visibility
        );
    }

    /// Renamed export: `function foo() {} export { foo as bar }`
    #[test]
    fn test_export_clause_renamed_export_updates_visibility() {
        let content = "function foo() { return 1; }\n\nexport { foo as bar };\n";
        let result = parse_js("export_rename.js", content);

        let foo = result
            .symbols
            .iter()
            .find(|s| s.name == "foo")
            .expect("Must find function 'foo'");
        assert_eq!(
            foo.visibility,
            Visibility::Public,
            "export {{ foo as bar }} must set foo's visibility to Public"
        );
    }

    /// Multiple named exports: `export { a, b, c }`
    #[test]
    fn test_export_clause_multiple_names_all_public() {
        let content = "function a() {}\nfunction b() {}\nfunction c() {}\n\nexport { a, b, c };\n";
        let result = parse_js("export_multi.js", content);

        for name in &["a", "b", "c"] {
            let sym = result
                .symbols
                .iter()
                .find(|s| s.name == *name)
                .unwrap_or_else(|| panic!("Must find function '{}'", name));
            assert_eq!(
                sym.visibility,
                Visibility::Public,
                "export {{ a, b, c }} must set {}'s visibility to Public",
                name
            );
        }
    }

    /// Graceful handling: `export { nonexistent }` — no matching symbol.
    #[test]
    fn test_export_clause_nonexistent_name_no_panic() {
        let content = "function realFn() {}\n\nexport { nonexistent };\n";
        let result = parse_js("export_missing.js", content);

        let real = result
            .symbols
            .iter()
            .find(|s| s.name == "realFn")
            .expect("Must find function 'realFn'");
        assert_eq!(
            real.visibility,
            Visibility::Private,
            "realFn must remain Private — it's not in the export clause"
        );
    }

    /// Arrow function via export clause: `const fn = () => {}; export { fn }`
    #[test]
    fn test_export_clause_arrow_function_becomes_public() {
        let content = "const process = (data) => data.map(x => x * 2);\n\nexport { process };\n";
        let result = parse_js("export_arrow_clause.js", content);

        let p = result
            .symbols
            .iter()
            .find(|s| s.name == "process")
            .expect("Must find arrow function 'process'");
        assert_eq!(
            p.visibility,
            Visibility::Public,
            "Arrow function in export clause must become Public"
        );
    }

    /// Regression: inline exports must still work after adding export clause handling.
    #[test]
    fn test_export_inline_still_works_after_clause_support() {
        let content = "export function inlined() { return 42; }\n";
        let result = parse_js("export_inline_regression.js", content);

        let f = result
            .symbols
            .iter()
            .find(|s| s.name == "inlined")
            .expect("Must find function 'inlined'");
        assert_eq!(
            f.visibility,
            Visibility::Public,
            "Inline export must still produce Public — regression guard"
        );
    }

    /// Re-export must NOT crash: `export { foo } from './other'`
    #[test]
    fn test_reexport_does_not_panic() {
        let content = "function foo() {}\n\nexport { foo } from './other';\n";
        let result = parse_js("reexport.js", content);
        let _result = result; // Parse must succeed without panic
    }

    /// Export clause before declaration (legal but unusual JS).
    #[test]
    fn test_export_clause_before_declaration() {
        let content = "export { foo };\n\nfunction foo() { return 1; }\n";
        let result = parse_js("export_before_decl.js", content);

        let foo = result.symbols.iter().find(|s| s.name == "foo");
        assert!(
            foo.is_some(),
            "Function 'foo' must be extracted regardless of export clause order"
        );
    }

    // -----------------------------------------------------------------------
    // Cycle 5 — Entity ID uniqueness (D4)
    // -----------------------------------------------------------------------

    /// JS adapter: qualified name must include extension.
    #[test]
    fn test_js_qualified_name_includes_extension() {
        let content = "function hello() { return 'hi'; }\n";
        let result = parse_js("app.js", content);

        let hello = result
            .symbols
            .iter()
            .find(|s| s.name == "hello")
            .expect("Must find function 'hello'");

        assert!(
            hello.qualified_name.starts_with("app.js::"),
            "JS qualified name must start with 'app.js::' (includes extension), got: {}",
            hello.qualified_name
        );
    }

    // =========================================================================
    // QA-2: Recursion Depth Protection — JavaScript Adapter (Cycle 7)
    // =========================================================================

    #[test]
    fn test_js_deeply_nested_callbacks_no_crash() {
        let adapter = JsAdapter::new();
        let mut code = String::new();
        for i in 0..500 {
            code.push_str(&format!("function f{i}() {{ "));
        }
        code.push_str("console.log('deep');");
        for _ in 0..500 {
            code.push_str(" }");
        }
        let path = Path::new("deep_callback.js");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on 500-deep nested functions"
        );
        let parsed = result.unwrap();
        assert!(!parsed.symbols.is_empty(), "Must extract shallow functions");
    }

    #[test]
    fn test_js_nested_arrow_functions_no_crash() {
        let adapter = JsAdapter::new();
        let mut code = String::new();
        for i in 0..300 {
            code.push_str(&format!("const a{i} = () => {{ "));
        }
        code.push_str("return 1;");
        for _ in 0..300 {
            code.push_str(" };");
        }
        let path = Path::new("deep_arrow.js");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on 300-deep nested arrow functions"
        );
    }

    #[test]
    fn test_js_nested_class_expressions_no_crash() {
        let adapter = JsAdapter::new();
        let mut code = String::new();
        for i in 0..300 {
            code.push_str(&format!("class C{i} {{ m{i}() {{ "));
        }
        code.push_str("return null;");
        for _ in 0..300 {
            code.push_str(" } }");
        }
        let path = Path::new("deep_class.js");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on 300-deep nested class declarations"
        );
    }

    #[test]
    fn test_js_10k_expression_nesting_no_crash() {
        let adapter = JsAdapter::new();
        let inner = "1 + ".repeat(10_000);
        let code = format!("function f() {{ let x = {}1; }}", inner);
        let path = Path::new("deep_js_expr.js");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on 10K-deep JS expression tree"
        );
        let parsed = result.unwrap();
        let has_fn = parsed.symbols.iter().any(|s| s.name == "f");
        assert!(has_fn, "Top-level function must be extracted");
    }

    #[test]
    fn test_js_partial_results_before_depth_limit() {
        let adapter = JsAdapter::new();
        let mut code = String::from("export function topLevel() {}\n");
        for i in 0..400 {
            code.push_str(&format!("function nested{i}() {{ "));
        }
        code.push_str("return 1;");
        for _ in 0..400 {
            code.push_str(" }");
        }
        let path = Path::new("js_partial.js");
        let result = adapter.parse_file(path, &code).unwrap();
        let names: Vec<&str> = result.symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"topLevel"),
            "Exported top-level function must survive depth limiting"
        );
    }

    #[test]
    fn test_js_mixed_nesting_no_crash() {
        let adapter = JsAdapter::new();
        let mut code = String::new();
        for i in 0..150 {
            code.push_str(&format!("function f{i}() {{ class C{i} {{ m{i}() {{ "));
        }
        code.push_str("return 42;");
        for _ in 0..150 {
            code.push_str(" } } }");
        }
        let path = Path::new("js_mixed.js");
        let result = adapter.parse_file(path, &code);
        assert!(
            result.is_ok(),
            "Must not crash on mixed function/class nesting"
        );
        let parsed = result.unwrap();
        let fn_count = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        let class_count = parsed
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .count();
        assert!(fn_count > 0, "Must extract some functions");
        assert!(class_count > 0, "Must extract some classes");
    }

    // =========================================================================
    // QA-2: Duplicate Entity IDs — Getter/Setter Fix (Cycle 7)
    // =========================================================================

    #[test]
    fn test_js_getter_only_distinct_name() {
        let adapter = JsAdapter::new();
        let code = r#"
class Config {
    get value() {
        return this._value;
    }
}
"#;
        let result = adapter.parse_file(Path::new("config.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"get_value"),
            "Getter must have 'get_' prefix, got: {:?}",
            method_names
        );
    }

    #[test]
    fn test_js_setter_only_distinct_name() {
        let adapter = JsAdapter::new();
        let code = r#"
class Config {
    set value(v) {
        this._value = v;
    }
}
"#;
        let result = adapter.parse_file(Path::new("config.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"set_value"),
            "Setter must have 'set_' prefix, got: {:?}",
            method_names
        );
    }

    #[test]
    fn test_js_getter_and_setter_distinct_ids() {
        let adapter = JsAdapter::new();
        let code = r#"
class Config {
    get value() { return this._value; }
    set value(v) { this._value = v; }
}
"#;
        let result = adapter.parse_file(Path::new("config.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"get_value"),
            "Getter must be present"
        );
        assert!(
            method_names.contains(&"set_value"),
            "Setter must be present"
        );
        assert_eq!(
            method_names.iter().filter(|n| n.contains("value")).count(),
            2,
            "Must have exactly 2 distinct entities for getter+setter"
        );
        let qualified: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.qualified_name.as_str())
            .collect();
        let unique: std::collections::HashSet<&&str> = qualified.iter().collect();
        assert_eq!(
            unique.len(),
            qualified.len(),
            "All qualified names must be unique"
        );
    }

    #[test]
    fn test_js_getter_setter_regular_method_same_name() {
        let adapter = JsAdapter::new();
        let code = r#"
class Weird {
    get x() { return 1; }
    set x(v) { this._x = v; }
    x() { return "method"; }
}
"#;
        let result = adapter.parse_file(Path::new("weird.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(method_names.contains(&"get_x"), "Getter must be 'get_x'");
        assert!(method_names.contains(&"set_x"), "Setter must be 'set_x'");
        assert!(method_names.contains(&"x"), "Regular method must be 'x'");
    }

    #[test]
    fn test_js_regular_method_starting_with_get_not_prefixed() {
        let adapter = JsAdapter::new();
        let code = r#"
class UserService {
    getUser(id) {
        return this.db.find(id);
    }
    setDefaults() {
        this.defaults = {};
    }
}
"#;
        let result = adapter.parse_file(Path::new("service.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"getUser"),
            "Regular method 'getUser' must NOT become 'get_getUser', got: {:?}",
            method_names
        );
        assert!(
            method_names.contains(&"setDefaults"),
            "Regular method 'setDefaults' must NOT become 'set_setDefaults', got: {:?}",
            method_names
        );
    }

    // =========================================================================
    // QA-2: Regression Guards (Cycle 7)
    // =========================================================================

    #[test]
    fn test_js_class_methods_still_extracted_after_getter_setter_fix() {
        let adapter = JsAdapter::new();
        let code = r#"
class MyClass {
    constructor() { this.x = 1; }
    normalMethod() { return this.x; }
    async asyncMethod() { return await fetch('/'); }
}
"#;
        let result = adapter.parse_file(Path::new("my_class.js"), code).unwrap();
        let method_names: Vec<&str> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Method)
            .map(|s| s.name.as_str())
            .collect();
        assert!(
            method_names.contains(&"constructor"),
            "constructor must be extracted"
        );
        assert!(
            method_names.contains(&"normalMethod"),
            "normalMethod must be extracted"
        );
    }

    // =========================================================================
    // QA-1: TypeScript Entity Extraction — Cycle 17
    // =========================================================================

    fn symbols_by_kind(result: &ParseResult, kind: SymbolKind) -> Vec<&Symbol> {
        result.symbols.iter().filter(|s| s.kind == kind).collect()
    }

    // Section 1: Interface Extraction (TS-1 through TS-5)

    #[test]
    fn ts_interface_declaration_extracted() {
        let content = r#"interface User {
    name: string;
    age: number;
    email: string;
}"#;
        let result = parse_js("types.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "User"),
            "Must extract Interface 'User', got: {:?}",
            ifaces.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let user = ifaces.iter().find(|s| s.name == "User").unwrap();
        assert_eq!(user.visibility, Visibility::Private);
    }

    #[test]
    fn ts_exported_interface_extracted_as_public() {
        let content = r#"export interface Config {
    host: string;
    port: number;
}"#;
        let result = parse_js("config.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        let cfg = ifaces
            .iter()
            .find(|s| s.name == "Config")
            .expect("Must extract Interface 'Config'");
        assert_eq!(cfg.visibility, Visibility::Public);
    }

    #[test]
    fn ts_interface_with_methods_extracted() {
        let content = r#"interface Repository {
    find(id: string): User;
    save(entity: User): void;
    delete(id: string): boolean;
}"#;
        let result = parse_js("service.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "Repository"),
            "Must extract Interface 'Repository'"
        );
        // Method signatures should NOT produce separate Method symbols
        let methods = symbols_by_kind(&result, SymbolKind::Method);
        assert_eq!(
            methods.len(),
            0,
            "Interface method signatures must not produce Method symbols"
        );
    }

    #[test]
    fn ts_multiple_interfaces_all_extracted() {
        let content = r#"interface Readable {
    read(): string;
}

interface Writable {
    write(data: string): void;
}

interface Closable {
    close(): void;
}"#;
        let result = parse_js("models.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        let names: Vec<&str> = ifaces.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Readable"),
            "Missing Readable in {:?}",
            names
        );
        assert!(
            names.contains(&"Writable"),
            "Missing Writable in {:?}",
            names
        );
        assert!(
            names.contains(&"Closable"),
            "Missing Closable in {:?}",
            names
        );
    }

    #[test]
    fn ts_interface_extends_extracted() {
        let content = r#"interface User {
    name: string;
}

interface Admin extends User {
    permissions: string[];
    role: string;
}"#;
        let result = parse_js("admin.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert_eq!(
            ifaces.len(),
            2,
            "Must extract 2 interfaces, got {}",
            ifaces.len()
        );
    }

    // Section 2: Enum Extraction (TS-6 through TS-9)

    #[test]
    fn ts_enum_declaration_extracted() {
        let content = r#"enum Status {
    Active,
    Inactive,
    Pending
}"#;
        let result = parse_js("status.ts", content);
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(
            enums.iter().any(|s| s.name == "Status"),
            "Must extract Enum 'Status'"
        );
    }

    #[test]
    fn ts_enum_with_values_extracted() {
        let content = r#"enum HttpCode {
    OK = 200,
    NotFound = 404,
    InternalError = 500
}"#;
        let result = parse_js("http.ts", content);
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(enums.iter().any(|s| s.name == "HttpCode"));
    }

    #[test]
    fn ts_const_enum_exported() {
        let content = r#"export const enum Direction {
    Up = "UP",
    Down = "DOWN",
    Left = "LEFT",
    Right = "RIGHT"
}"#;
        let result = parse_js("direction.ts", content);
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        let dir = enums
            .iter()
            .find(|s| s.name == "Direction")
            .expect("Must extract Enum 'Direction'");
        assert_eq!(dir.visibility, Visibility::Public);
    }

    #[test]
    fn ts_enum_string_values_with_special_chars() {
        let content = r#"enum Template {
    Header = "{ header }",
    Footer = "} footer {",
    Body = "normal"
}"#;
        let result = parse_js("templates.ts", content);
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(
            enums.iter().any(|s| s.name == "Template"),
            "Must extract Enum 'Template' despite braces in string values"
        );
    }

    // Section 3: Type Alias Extraction (TS-10 through TS-12)

    #[test]
    fn ts_type_alias_extracted() {
        let content = "type UserId = string;\n";
        let result = parse_js("aliases.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "UserId"),
            "Must extract type alias 'UserId' as Interface"
        );
    }

    #[test]
    fn ts_union_type_alias_extracted() {
        let content = r#"type Result<T> =
    | { ok: true; value: T }
    | { ok: false; error: Error };"#;
        let result = parse_js("result.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "Result"),
            "Must extract union type alias 'Result'"
        );
    }

    #[test]
    fn ts_exported_type_alias() {
        let content = "export type Callback = (data: string) => void;\n";
        let result = parse_js("callbacks.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        let cb = ifaces
            .iter()
            .find(|s| s.name == "Callback")
            .expect("Must extract type alias 'Callback'");
        assert_eq!(cb.visibility, Visibility::Public);
    }

    // Section 4: Type Annotation Stripping (TS-13 through TS-16)

    #[test]
    fn ts_typed_function_params_extracted() {
        let content = r#"function greet(name: string, age: number): string {
    return `Hello ${name}, age ${age}`;
}"#;
        let result = parse_js("greet.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "greet"),
            "Must extract Function 'greet' despite type annotations"
        );
    }

    #[test]
    fn ts_typed_arrow_function_extracted() {
        let content = r#"const transform = (input: string): number => {
    return parseInt(input, 10);
};"#;
        let result = parse_js("transform.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "transform"),
            "Must extract arrow Function 'transform' despite type annotations"
        );
    }

    #[test]
    fn ts_generic_function_extracted() {
        let content = r#"function identity<T>(value: T): T {
    return value;
}"#;
        let result = parse_js("identity.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "identity"),
            "Must extract generic Function 'identity'"
        );
    }

    #[test]
    fn ts_complex_generic_constraints() {
        let content = r#"function merge<T extends object, U extends object>(a: T, b: U): T & U {
    return { ...a, ...b };
}"#;
        let result = parse_js("merge.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "merge"),
            "Must extract Function 'merge' with complex generics"
        );
    }

    // Section 5: Class with TS-Specific Features (TS-17 through TS-20)

    #[test]
    fn ts_class_access_modifiers() {
        let content = r#"class Account {
    private balance: number = 0;

    public getBalance(): number {
        return this.balance;
    }

    protected deposit(amount: number): void {
        this.balance += amount;
    }

    private reset(): void {
        this.balance = 0;
    }
}"#;
        let result = parse_js("account.ts", content);
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "Account"),
            "Must extract Class 'Account'"
        );
        let methods = symbols_by_kind(&result, SymbolKind::Method);
        let method_names: Vec<&str> = methods.iter().map(|s| s.name.as_str()).collect();
        assert!(
            method_names.contains(&"getBalance"),
            "Must extract method 'getBalance', got {:?}",
            method_names
        );
        assert!(
            method_names.contains(&"deposit"),
            "Must extract method 'deposit', got {:?}",
            method_names
        );
        assert!(
            method_names.contains(&"reset"),
            "Must extract method 'reset', got {:?}",
            method_names
        );
    }

    #[test]
    fn ts_class_implements_interface() {
        let content = r#"interface Logger {
    log(message: string): void;
    error(message: string): void;
}

class ConsoleLogger implements Logger {
    log(message: string): void {
        console.log(message);
    }
    error(message: string): void {
        console.error(message);
    }
}"#;
        let result = parse_js("service.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(ifaces.iter().any(|s| s.name == "Logger"));
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(classes.iter().any(|s| s.name == "ConsoleLogger"));
        let methods = symbols_by_kind(&result, SymbolKind::Method);
        assert!(methods.len() >= 2, "Must extract at least 2 methods");
    }

    #[test]
    fn ts_abstract_class_extracted() {
        let content = r#"abstract class Shape {
    abstract area(): number;

    describe(): string {
        return `Area: ${this.area()}`;
    }
}"#;
        let result = parse_js("shape.ts", content);
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "Shape"),
            "Must extract abstract Class 'Shape'"
        );
    }

    #[test]
    fn ts_constructor_parameter_properties() {
        let content = r#"class Point {
    constructor(
        private readonly x: number,
        private readonly y: number
    ) {}

    distanceTo(other: Point): number {
        return Math.sqrt((this.x - other.x) ** 2 + (this.y - other.y) ** 2);
    }
}"#;
        let result = parse_js("point.ts", content);
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(classes.iter().any(|s| s.name == "Point"));
    }

    // Section 6: Import/Export Patterns (TS-21 through TS-23)

    #[test]
    fn ts_type_only_import_extracted() {
        let content = r#"import type { User } from './models';
import { validate } from './utils';

function processUser(user: User): boolean {
    return validate(user);
}"#;
        let result = parse_js("handler.ts", content);
        let imports: Vec<_> = result
            .references
            .iter()
            .filter(|r| r.kind == ReferenceKind::Import)
            .collect();
        assert!(
            imports.len() >= 2,
            "Must extract at least 2 imports, got {}",
            imports.len()
        );
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "processUser"));
    }

    #[test]
    fn ts_type_reexport() {
        let content = "export type { Config } from './config';\n";
        let result = parse_js("index.ts", content);
        // Should not panic and should handle gracefully
        let _ = result;
    }

    #[test]
    fn ts_namespace_import_with_types() {
        let content = r#"import * as models from './models';

function lookup(m) {
    return m;
}"#;
        let result = parse_js("app.ts", content);
        // Namespace imports are standard JS — verify no panic
        let _ = result;
    }

    // Section 7: Regression Guards (REG-1 through REG-5)

    #[test]
    fn ts_file_with_pure_js_extracts_all() {
        let content = r#"function add(a, b) {
    return a + b;
}

function multiply(a, b) {
    return a * b;
}

class Calculator {
    compute(op, a, b) {
        if (op === '+') return add(a, b);
        return multiply(a, b);
    }
}"#;
        let result = parse_js("utils.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "add"), "Must extract 'add'");
        assert!(
            fns.iter().any(|s| s.name == "multiply"),
            "Must extract 'multiply'"
        );
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "Calculator"),
            "Must extract 'Calculator'"
        );
    }

    #[test]
    fn js_file_not_preprocessed() {
        let content = r#"// This interface is implemented in Java
function createInterface(name) {
    return { type: name };
}

class Builder {
    build() {
        return createInterface("enum");
    }
}"#;
        let result = parse_js("legacy.js", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "createInterface"));
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(classes.iter().any(|s| s.name == "Builder"));
        let methods = symbols_by_kind(&result, SymbolKind::Method);
        assert!(methods.iter().any(|s| s.name == "build"));
        // No interfaces should be extracted from .js file
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert_eq!(
            ifaces.len(),
            0,
            "JS file must not produce Interface symbols"
        );
    }

    #[test]
    fn jsx_file_not_preprocessed() {
        let content = r#"function Component() {
    return <div>Hello</div>;
}"#;
        let result = parse_js("component.jsx", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "Component"));
    }

    #[test]
    fn ts_empty_file_produces_empty_result() {
        let result = parse_js("empty.ts", "");
        assert_eq!(
            result.symbols.len(),
            0,
            "Empty .ts file must produce 0 symbols"
        );
        assert!(result.scopes.len() >= 1, "Must have at least file scope");
    }

    #[test]
    fn cjs_file_not_preprocessed() {
        let content = "function helper() { return 1; }\nmodule.exports = { helper };\n";
        let result = parse_js("module.cjs", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "helper"));
    }

    // Section 8: Adversarial Edge Cases (ADV-1 through ADV-8)

    #[test]
    fn ts_syntax_in_strings_not_stripped() {
        let content = r#"function parseType(input: string): string {
    const pattern = "interface Foo { bar: string }";
    return pattern;
}"#;
        let result = parse_js("strings.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "parseType"),
            "Must extract 'parseType'"
        );
        // The string content must NOT produce an Interface symbol named "Foo"
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            !ifaces.iter().any(|s| s.name == "Foo"),
            "Must NOT extract Interface 'Foo' from string literal"
        );
    }

    #[test]
    fn ts_syntax_in_comments_not_stripped() {
        let content = r#"// interface OldApi { deprecated: true }
/* enum Status {
   Active,
   Inactive
} */
function helper(): void {
    return;
}"#;
        let result = parse_js("documented.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(fns.iter().any(|s| s.name == "helper"));
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            !ifaces.iter().any(|s| s.name == "OldApi"),
            "Must NOT extract Interface from comment"
        );
    }

    #[test]
    fn ts_nested_generics_fully_stripped() {
        let content = r#"function transform<T>(data: Map<string, Array<T>>): Map<string, T[]> {
    const result = new Map();
    return result;
}"#;
        let result = parse_js("nested.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "transform"),
            "Must extract Function 'transform' with nested generics"
        );
    }

    #[test]
    fn ts_interleaved_with_js_extracts_both() {
        let content = r#"interface Config {
    debug: boolean;
}

function create(cfg: Config): void {
    console.log(cfg);
}

enum Level {
    Info,
    Warn,
    Error
}

class App {
    run() {
        create({ debug: true });
    }
}"#;
        let result = parse_js("mixed.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "Config"),
            "Must extract Interface 'Config'"
        );
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "create"),
            "Must extract Function 'create'"
        );
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(
            enums.iter().any(|s| s.name == "Level"),
            "Must extract Enum 'Level'"
        );
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "App"),
            "Must extract Class 'App'"
        );
    }

    #[test]
    fn tsx_jsx_and_generics_coexist() {
        let content = r#"function Wrapper<T>(props: { data: T }) {
    return <div>{JSON.stringify(props.data)}</div>;
}"#;
        // Note: JSX `<div>` may cause issues, but the function should still extract
        let result = parse_js("component.tsx", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "Wrapper"),
            "Must extract Function 'Wrapper' from .tsx file"
        );
    }

    #[test]
    fn ts_interface_nested_braces() {
        let content = r#"interface DataStore {
    query(sql: string): { rows: any[]; count: number };
    metadata(): { version: string; tables: { name: string }[] };
}"#;
        let result = parse_js("complex.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "DataStore"),
            "Must extract Interface 'DataStore' with nested braces"
        );
    }

    #[test]
    fn ts_declare_function_extracted() {
        let content = r#"declare function fetch(url: string): Promise<Response>;
declare class EventEmitter {
    on(event: string, handler: Function): void;
}"#;
        let result = parse_js("ambient.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        assert!(
            fns.iter().any(|s| s.name == "fetch"),
            "Must extract declare function 'fetch'"
        );
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "EventEmitter"),
            "Must extract declare class 'EventEmitter'"
        );
    }

    #[test]
    fn ts_only_constructs_all_extracted() {
        let content = r#"interface Serializable {
    serialize(): string;
    deserialize(data: string): void;
}

enum Format {
    JSON,
    XML,
    CSV
}

type Options = {
    format: Format;
    pretty: boolean;
};"#;
        let result = parse_js("pure_types.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        assert!(
            ifaces.iter().any(|s| s.name == "Serializable"),
            "Must extract Interface 'Serializable'"
        );
        assert!(
            ifaces.iter().any(|s| s.name == "Options"),
            "Must extract type alias 'Options' as Interface"
        );
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(
            enums.iter().any(|s| s.name == "Format"),
            "Must extract Enum 'Format'"
        );
    }

    // Section 9: Position Accuracy (POS-1 through POS-3)

    #[test]
    fn ts_function_line_numbers_correct() {
        let content = "interface Unused {\n    x: number;\n}\n\nfunction target(a: string, b: number): boolean {\n    return a.length > b;\n}\n";
        let result = parse_js("positioned.ts", content);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        let target = fns
            .iter()
            .find(|s| s.name == "target")
            .expect("Must find 'target'");
        assert_eq!(
            target.location.line, 5,
            "Function 'target' must be at line 5, got {}",
            target.location.line
        );
    }

    #[test]
    fn ts_interface_line_number_correct() {
        let content = "interface First {\n    x: number;\n}\n";
        let result = parse_js("first.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        let first = ifaces
            .iter()
            .find(|s| s.name == "First")
            .expect("Must find 'First'");
        assert_eq!(
            first.location.line, 1,
            "Interface 'First' must be at line 1, got {}",
            first.location.line
        );
    }

    #[test]
    fn ts_entity_ordering_preserved() {
        let content = "interface Alpha {\n    x: number;\n}\n\nfunction beta(a: string): void {\n    return;\n}\n\nenum Gamma {\n    A,\n    B\n}\n";
        let result = parse_js("ordered.ts", content);
        let ifaces = symbols_by_kind(&result, SymbolKind::Interface);
        let fns = symbols_by_kind(&result, SymbolKind::Function);
        let enums = symbols_by_kind(&result, SymbolKind::Enum);
        assert!(!ifaces.is_empty() && !fns.is_empty() && !enums.is_empty());
        let alpha_line = ifaces
            .iter()
            .find(|s| s.name == "Alpha")
            .unwrap()
            .location
            .line;
        let beta_line = fns.iter().find(|s| s.name == "beta").unwrap().location.line;
        let gamma_line = enums
            .iter()
            .find(|s| s.name == "Gamma")
            .unwrap()
            .location
            .line;
        assert!(
            alpha_line < beta_line,
            "Alpha ({}) must come before beta ({})",
            alpha_line,
            beta_line
        );
        assert!(
            beta_line < gamma_line,
            "beta ({}) must come before Gamma ({})",
            beta_line,
            gamma_line
        );
    }

    // Section 10: Decorator Handling (DEC-1 through DEC-2)

    #[test]
    fn ts_decorated_class_extracted() {
        let content = r#"function Injectable() {
    return function(target) { return target; };
}

@Injectable()
class Service {
    handle() {}
}"#;
        let result = parse_js("service.ts", content);
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(
            classes.iter().any(|s| s.name == "Service"),
            "Must extract decorated Class 'Service'"
        );
    }

    #[test]
    fn ts_multiple_decorators() {
        let content = r#"function Dec1() { return function(t) { return t; }; }
function Dec2() { return function(t) { return t; }; }

@Dec1()
@Dec2()
class Controller {
    getUsers() { return []; }
}"#;
        let result = parse_js("controller.ts", content);
        let classes = symbols_by_kind(&result, SymbolKind::Class);
        assert!(classes.iter().any(|s| s.name == "Controller"));
        let methods = symbols_by_kind(&result, SymbolKind::Method);
        assert!(methods.iter().any(|s| s.name == "getUsers"));
    }

    // -----------------------------------------------------------------------
    // C18 QA-1: Entity Deduplication Verification (DEDUP-1 through DEDUP-10)
    // -----------------------------------------------------------------------

    #[test]
    fn ts_function_not_duplicated_after_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "function greet(name: string): void {\n  console.log(name);\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "greet")
            .collect();
        assert_eq!(
            funcs.len(),
            1,
            "Expected exactly 1 'greet' function, got {} — dedup fix may not be applied",
            funcs.len()
        );
    }

    #[test]
    fn ts_class_not_duplicated_after_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "class Foo implements Bar {\n  constructor() {}\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "Foo")
            .collect();
        assert_eq!(
            classes.len(),
            1,
            "Expected exactly 1 'Foo' class, got {} — dedup fix may not be applied",
            classes.len()
        );
    }

    #[test]
    fn ts_interface_still_extracted_after_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "interface User {\n  name: string;\n  age: number;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let ifaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface && s.name == "User")
            .collect();
        assert_eq!(
            ifaces.len(),
            1,
            "Interface must still be extracted via pre-extraction after dedup fix"
        );
    }

    #[test]
    fn ts_enum_still_extracted_after_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "enum Direction {\n  Up,\n  Down,\n  Left,\n  Right\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum && s.name == "Direction")
            .collect();
        assert_eq!(
            enums.len(),
            1,
            "Enum must still be extracted via pre-extraction after dedup fix"
        );
    }

    #[test]
    fn ts_type_alias_still_extracted_after_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "type StringOrNumber = string | number;\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let types: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "StringOrNumber")
            .collect();
        assert_eq!(
            types.len(),
            1,
            "Type alias must still be extracted via pre-extraction after dedup fix"
        );
    }

    #[test]
    fn ts_mixed_file_exact_entity_counts() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("mixed.ts");
        let content = "interface Config {\n  debug: boolean;\n}\n\n\
                        function setup(cfg: Config): void {\n  console.log(cfg);\n}\n\n\
                        class App implements Config {\n  debug: boolean = false;\n  constructor() {}\n}\n\n\
                        enum Mode {\n  Dev,\n  Prod\n}\n\n\
                        type AppConfig = Config & { mode: Mode };\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let interfaces = result
            .symbols
            .iter()
            .filter(|s| {
                s.kind == SymbolKind::Interface
                    && !s.annotations.contains(&"type_alias".to_string())
            })
            .count();
        let functions = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        let classes = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .count();
        let enums = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .count();
        let type_aliases = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"type_alias".to_string()))
            .count();

        assert_eq!(
            interfaces, 1,
            "Expected 1 interface (Config), got {}",
            interfaces
        );
        assert_eq!(
            functions, 1,
            "Expected 1 function (setup), got {} — dedup fix may be incomplete",
            functions
        );
        assert_eq!(
            classes, 1,
            "Expected 1 class (App), got {} — dedup fix may be incomplete",
            classes
        );
        assert_eq!(enums, 1, "Expected 1 enum (Mode), got {}", enums);
        assert_eq!(
            type_aliases, 1,
            "Expected 1 type alias (AppConfig), got {}",
            type_aliases
        );
    }

    #[test]
    fn ts_export_function_not_duplicated() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content =
            "export function calculate(a: number, b: number): number {\n  return a + b;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "calculate")
            .collect();
        assert_eq!(funcs.len(), 1, "Export function should not be duplicated");
        assert_eq!(
            funcs[0].visibility,
            Visibility::Public,
            "Exported function must be Public"
        );
    }

    #[test]
    fn ts_export_class_not_duplicated() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "export class Service {\n  start(): void {}\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let classes: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "Service")
            .collect();
        assert_eq!(classes.len(), 1, "Export class should not be duplicated");
    }

    #[test]
    fn ts_declare_function_bodyless_not_lost() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "declare function greet(): void;\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let greets: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "greet")
            .collect();
        assert!(
            !greets.is_empty(),
            "declare function bodyless must still produce an entity"
        );
        assert!(
            greets.len() <= 1,
            "declare function must not produce duplicates, got {}",
            greets.len()
        );
    }

    #[test]
    fn ts_declare_class_bodyless_not_lost() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "declare class Foo {}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let foos: Vec<_> = result.symbols.iter().filter(|s| s.name == "Foo").collect();
        assert!(
            !foos.is_empty(),
            "declare class must still produce an entity after dedup fix"
        );
    }

    // -----------------------------------------------------------------------
    // C18 QA-1: Regression Guards (REG-1 through REG-5)
    // -----------------------------------------------------------------------

    #[test]
    fn ts_pure_js_in_ts_file_still_works() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("legacy.ts");
        let content = "function add(a, b) {\n  return a + b;\n}\n\nclass Animal {\n  constructor(name) {\n    this.name = name;\n  }\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "add")
            .count();
        let classes = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "Animal")
            .count();
        assert_eq!(
            funcs, 1,
            "Pure JS function in .ts file must produce exactly 1 entity"
        );
        assert_eq!(
            classes, 1,
            "Pure JS class in .ts file must produce exactly 1 entity"
        );
    }

    #[test]
    fn js_file_unaffected_by_dedup_fix() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("module.js");
        let content =
            "function process(data) {\n  return data;\n}\n\nclass Handler {\n  handle() {}\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "process")
            .count();
        let classes = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class && s.name == "Handler")
            .count();
        assert_eq!(funcs, 1, ".js file should produce exactly 1 function");
        assert_eq!(classes, 1, ".js file should produce exactly 1 class");
    }

    #[test]
    fn tsx_file_gets_preprocessing() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("component.tsx");
        let content = "interface Props {\n  label: string;\n}\n\nfunction Component(props: Props): void {\n  return;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let ifaces = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .count();
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function && s.name == "Component")
            .count();
        assert_eq!(
            ifaces, 1,
            ".tsx must trigger preprocessing and extract interfaces"
        );
        assert_eq!(funcs, 1, ".tsx function must not be duplicated");
    }

    #[test]
    fn jsx_file_no_preprocessing() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("component.jsx");
        let content = "function Component(props) {\n  return null;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        assert!(!is_typescript_file(&PathBuf::from("component.jsx")));
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        assert_eq!(funcs, 1);
    }

    // REG-5 is a meta-test: all existing 37 QA-1 C17 TS tests must still pass.
    // Verified by running `cargo test ts_` — no new code needed.

    // -----------------------------------------------------------------------
    // C18 QA-1: Fixture File Integration (FIX-1 through FIX-5)
    // -----------------------------------------------------------------------

    #[test]
    fn fixture_ts_file_routes_through_preprocessing() {
        assert!(is_typescript_file(&PathBuf::from(
            "tests/fixtures/typescript/interfaces.ts"
        )));
        assert!(is_typescript_file(&PathBuf::from(
            "tests/fixtures/typescript/typed_functions.ts"
        )));
        assert!(is_typescript_file(&PathBuf::from(
            "tests/fixtures/typescript/mixed.ts"
        )));
    }

    #[test]
    fn fixture_interfaces_matches_inline() {
        let adapter = JsAdapter::new();
        let fixture_content = "interface User {\n  name: string;\n  age: number;\n}\n\n\
                               interface Admin extends User {\n  role: string;\n}\n\n\
                               interface Collection<T> {\n  items: T[];\n  add(item: T): void;\n}\n";
        let fixture_path = PathBuf::from("tests/fixtures/typescript/interfaces.ts");
        let result = adapter.parse_file(&fixture_path, fixture_content).unwrap();
        let ifaces: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Interface)
            .map(|s| s.name.clone())
            .collect();
        assert_eq!(ifaces.len(), 3, "Should extract 3 interfaces from fixture");
        assert!(ifaces.contains(&"User".to_string()));
        assert!(ifaces.contains(&"Admin".to_string()));
        assert!(ifaces.contains(&"Collection".to_string()));
    }

    #[test]
    fn fixture_typed_functions_no_duplication() {
        let adapter = JsAdapter::new();
        let fixture_content = "function add(a: number, b: number): number {\n  return a + b;\n}\n\n\
                               export function greet(name: string): void {\n  console.log(name);\n}\n\n\
                               function identity<T>(value: T): T {\n  return value;\n}\n";
        let fixture_path = PathBuf::from("tests/fixtures/typescript/typed_functions.ts");
        let result = adapter.parse_file(&fixture_path, fixture_content).unwrap();
        let funcs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .collect();
        assert_eq!(
            funcs.len(),
            3,
            "Should extract exactly 3 functions — no duplicates"
        );
    }

    #[test]
    fn fixture_mixed_ts_complete_coverage() {
        let adapter = JsAdapter::new();
        let fixture_content = "interface Config {\n  verbose: boolean;\n}\n\n\
                               function init(cfg: Config): void {\n  console.log(cfg);\n}\n\n\
                               class App {\n  constructor() {}\n}\n\n\
                               enum LogLevel {\n  Debug,\n  Info,\n  Error\n}\n\n\
                               type AppMode = 'dev' | 'prod';\n\n\
                               const VERSION = '1.0.0';\n";
        let fixture_path = PathBuf::from("tests/fixtures/typescript/mixed.ts");
        let result = adapter.parse_file(&fixture_path, fixture_content).unwrap();
        let interfaces = result
            .symbols
            .iter()
            .filter(|s| {
                s.kind == SymbolKind::Interface
                    && !s.annotations.contains(&"type_alias".to_string())
            })
            .count();
        let type_aliases = result
            .symbols
            .iter()
            .filter(|s| s.annotations.contains(&"type_alias".to_string()))
            .count();
        let functions = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        let classes = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Class)
            .count();
        let enums = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum)
            .count();
        assert_eq!(interfaces, 1, "1 interface (Config)");
        assert_eq!(type_aliases, 1, "1 type alias (AppMode)");
        assert_eq!(functions, 1, "1 function (init)");
        assert_eq!(classes, 1, "1 class (App)");
        assert_eq!(enums, 1, "1 enum (LogLevel)");
    }

    #[test]
    fn fixture_js_file_no_preprocessing() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("tests/fixtures/javascript/module.js");
        let content = "function helper() { return 42; }\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        assert_eq!(funcs, 1);
    }

    // -----------------------------------------------------------------------
    // C18 QA-1: Whitespace Artifact Tests (WS-1 through WS-5)
    // -----------------------------------------------------------------------

    #[test]
    fn ts_function_signature_no_multispace() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "function greet(name: string, age: number): void {\n  return;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let func = result
            .symbols
            .iter()
            .find(|s| s.kind == SymbolKind::Function && s.name == "greet")
            .expect("greet function should exist");
        if let Some(ref sig) = func.signature {
            assert!(
                !sig.contains("  "),
                "Signature '{}' contains multi-space runs — whitespace collapse not applied",
                sig
            );
        }
    }

    #[test]
    fn ts_class_method_signature_no_multispace() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "class Svc {\n  process(input: string, count: number): boolean {\n    return true;\n  }\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        for sym in &result.symbols {
            if let Some(ref sig) = sym.signature {
                assert!(
                    !sig.contains("  "),
                    "Symbol '{}' signature '{}' has multi-space artifact",
                    sym.name,
                    sig
                );
            }
        }
    }

    #[test]
    fn ts_arrow_function_signature_no_multispace() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content =
            "const transform = (data: string[]): number[] => {\n  return data.map(Number);\n};\n";
        let result = adapter.parse_file(&path, content).unwrap();
        for sym in &result.symbols {
            if let Some(ref sig) = sym.signature {
                assert!(
                    !sig.contains("  "),
                    "Signature '{}' has whitespace artifact",
                    sig
                );
            }
        }
    }

    #[test]
    fn ts_generic_signature_no_multispace() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "function merge<T extends object, U extends object>(a: T, b: U): T & U {\n  return Object.assign(a, b);\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "merge")
            .expect("merge function should exist");
        if let Some(ref sig) = func.signature {
            assert!(
                !sig.contains("  "),
                "Generic signature '{}' has multi-space artifact",
                sig
            );
        }
    }

    #[test]
    fn ts_many_params_signature_no_multispace() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "function configure(host: string, port: number, secure: boolean, timeout: number): void {\n  return;\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let func = result
            .symbols
            .iter()
            .find(|s| s.name == "configure")
            .expect("configure function should exist");
        if let Some(ref sig) = func.signature {
            assert!(
                !sig.contains("  "),
                "Multi-param signature '{}' has whitespace artifact",
                sig
            );
        }
    }

    // -----------------------------------------------------------------------
    // C18 QA-1: Adversarial Edge Cases (ADV-1 through ADV-5)
    // -----------------------------------------------------------------------

    #[test]
    fn ts_only_functions_no_preextraction_needed() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "function a(x: number): number { return x; }\n\
                        function b(y: string): void { console.log(y); }\n\
                        export function c(): boolean { return true; }\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let funcs = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Function)
            .count();
        assert_eq!(
            funcs, 3,
            "3 functions, each appearing exactly once (tree-sitter only, no pre-extraction)"
        );
    }

    #[test]
    fn ts_empty_file_no_crash() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("empty.ts");
        let content = "";
        let result = adapter.parse_file(&path, content).unwrap();
        let named: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name != "empty" && s.kind != SymbolKind::Module)
            .collect();
        assert!(
            named.is_empty(),
            "Empty .ts file should produce no named entities"
        );
    }

    #[test]
    fn ts_only_comments_no_entities() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("comments.ts");
        let content =
            "// This is a comment\n/* Block comment\n   spanning lines */\n/// Triple slash\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let named = result
            .symbols
            .iter()
            .filter(|s| s.kind != SymbolKind::Module)
            .count();
        assert_eq!(named, 0, "Comment-only .ts file should produce no entities");
    }

    #[test]
    fn ts_export_declare_function_handled() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("ambient.ts");
        let content = "export declare function fetch(url: string): Promise<Response>;\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let fetchs: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.name == "fetch")
            .collect();
        assert!(
            !fetchs.is_empty(),
            "export declare function must produce at least 1 entity"
        );
        assert!(
            fetchs.len() <= 1,
            "export declare function must not produce duplicates, got {}",
            fetchs.len()
        );
    }

    #[test]
    fn ts_const_enum_extracted() {
        let adapter = JsAdapter::new();
        let path = PathBuf::from("test.ts");
        let content = "const enum Status {\n  Active,\n  Inactive\n}\n";
        let result = adapter.parse_file(&path, content).unwrap();
        let enums: Vec<_> = result
            .symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::Enum && s.name == "Status")
            .collect();
        assert_eq!(
            enums.len(),
            1,
            "const enum should be extracted exactly once"
        );
    }
}
