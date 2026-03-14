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
            .map(|e| matches!(e, "js" | "jsx" | "ts" | "tsx" | "mjs"))
            .unwrap_or(false)
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParseResult, FlowspecError> {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_javascript::LANGUAGE.into())
            .map_err(|e| FlowspecError::Parse {
                file: path.to_path_buf(),
                reason: format!("failed to set JavaScript language: {}", e),
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
            false, // not inside export
        );

        // Extract all function/method calls from the AST
        extract_all_calls(&mut result, content_bytes, path, root);

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
fn visit_children(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
) {
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
        );
    }
}

/// Visit a single AST node and extract IR.
fn visit_node(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
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
            );
        }
        "export_statement" => {
            visit_export_statement(result, scope_stack, content, path, file_stem, node);
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
            );
        }
        "import_statement" => {
            extract_import(result, content, path, node);
        }
        _ => {
            // Recurse into other nodes
            visit_children(result, scope_stack, content, path, file_stem, node, false);
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
                );
            }
            "export_clause" => {
                // `export { foo, bar }` or `export { foo as bar }`
                // Skip re-exports: `export { foo } from './module'` — has a string source
                let has_source = {
                    let mut src_cursor = node.walk();
                    let result = node.children(&mut src_cursor).any(|c| c.kind() == "string");
                    result
                };
                if !has_source {
                    apply_export_clause_visibility(result, content, child);
                }
            }
            _ => {}
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
fn extract_function_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
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
        .map(|n| node_text(content, n).to_string());

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
        visit_children(result, scope_stack, content, path, file_stem, body, false);
    }

    scope_stack.pop();
}

/// Extract arrow functions from `lexical_declaration` / `variable_declaration`.
///
/// Walks `variable_declarator` children. If the value field is an `arrow_function`,
/// extracts it as a `SymbolKind::Function` with the variable name.
fn extract_arrow_functions_from_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
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
                        .map(|n| node_text(content, n).to_string());

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
                        visit_children(result, scope_stack, content, path, file_stem, body, false);
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
                        .map(|n| node_text(content, n).to_string());

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
                        visit_children(result, scope_stack, content, path, file_stem, body, false);
                    }

                    scope_stack.pop();
                }
            }
        }
    }
}

/// Extract a class declaration.
fn extract_class_declaration(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    exported: bool,
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
                extract_method_definition(result, scope_stack, content, path, file_stem, child);
            } else {
                visit_node(result, scope_stack, content, path, file_stem, child, false);
            }
        }
    }

    scope_stack.pop();
}

/// Extract a method definition inside a class body.
fn extract_method_definition(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
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
        .map(|n| node_text(content, n).to_string());

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

    if let Some(body) = node.child_by_field_name("body") {
        visit_children(result, scope_stack, content, path, file_stem, body, false);
    }

    scope_stack.pop();
}

/// Extract ES6 import statements.
///
/// Creates both a `Reference` (for graph edges) and a `Symbol` (for pattern detection)
/// for each imported name. The symbol uses `SymbolKind::Module` with an `"import"`
/// annotation so that `phantom_dependency` can find and check import symbols.
fn extract_import(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            // `import x from 'y'` — default import
            "identifier" => {
                let name = node_text(content, child).to_string();
                if !name.is_empty() && name != "import" && name != "from" {
                    add_import_symbol(result, &file_stem, &name, path, node);
                }
            }
            // `import { a, b } from 'y'` — named imports
            "import_clause" => {
                extract_import_clause(result, content, path, &file_stem, child);
            }
            // `import * as x from 'y'` — namespace import
            "namespace_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(content, name_node).to_string();
                    if !name.is_empty() {
                        add_import_symbol(result, &file_stem, &name, path, node);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Extract named imports from an import clause `{ a, b as c }`.
fn extract_import_clause(
    result: &mut ParseResult,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "identifier" => {
                let name = node_text(content, child).to_string();
                if !name.is_empty() && name != "import" && name != "from" {
                    add_import_symbol(result, file_stem, &name, path, child);
                }
            }
            "import_specifier" => {
                // Use alias if present, otherwise the name
                let sym_name = child
                    .child_by_field_name("alias")
                    .or_else(|| child.child_by_field_name("name"))
                    .map(|n| node_text(content, n).to_string())
                    .unwrap_or_default();
                if !sym_name.is_empty() {
                    add_import_symbol(result, file_stem, &sym_name, path, child);
                }
            }
            "named_imports" => {
                // Recurse into `{ a, b }`
                let mut inner_cursor = child.walk();
                for inner_child in child.children(&mut inner_cursor) {
                    if inner_child.kind() == "import_specifier" {
                        let sym_name = inner_child
                            .child_by_field_name("alias")
                            .or_else(|| inner_child.child_by_field_name("name"))
                            .map(|n| node_text(content, n).to_string())
                            .unwrap_or_default();
                        if !sym_name.is_empty() {
                            add_import_symbol(result, file_stem, &sym_name, path, inner_child);
                        }
                    }
                }
            }
            "namespace_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(content, name_node).to_string();
                    if !name.is_empty() {
                        add_import_symbol(result, file_stem, &name, path, child);
                    }
                }
            }
            _ => {}
        }
    }
}

/// Helper to add an import reference + symbol pair.
fn add_import_symbol(
    result: &mut ParseResult,
    file_stem: &str,
    name: &str,
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
        annotations: vec!["import".to_string()],
    });
}

/// Recursively walks the AST to extract all function/method call references.
///
/// Creates a `Reference` with `kind: ReferenceKind::Call` for each `call_expression`
/// node found. The callee name is stored in
/// `resolution: ResolutionStatus::Partial("call:<name>")` for later resolution
/// by `populate_graph`.
fn extract_all_calls(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    if node.kind() == "call_expression" {
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
        extract_all_calls(result, content, path, child);
    }
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
}
