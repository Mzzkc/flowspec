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
        extract_all_calls(&mut result, content_bytes, path, root);

        // Extract attribute accesses to track import usage (e.g., os.path.join, sys.argv)
        extract_attribute_accesses(&mut result, content_bytes, path, root);

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
        visit_node(result, scope_stack, content, path, file_stem, child, &[]);
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
    decorators: &[String],
) {
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
            );
        }
        "import_statement" => {
            extract_import(result, content, path, node);
        }
        "import_from_statement" => {
            extract_import_from(result, content, path, node);
        }
        "decorated_definition" => {
            extract_decorated(result, scope_stack, content, path, file_stem, node);
        }
        "expression_statement" => {
            // Check for assignment as direct child
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "assignment" && scope_stack.len() <= 1 {
                    extract_assignment(result, scope_stack, content, path, file_stem, child);
                }
            }
        }
        _ => {
            // Recurse into other nodes
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                visit_node(result, scope_stack, content, path, file_stem, child, &[]);
            }
        }
    }
}

/// Extract a function/method definition.
fn extract_function(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    decorators: &[String],
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
            visit_node(result, scope_stack, content, path, file_stem, child, &[]);
        }
    }

    scope_stack.pop();
}

/// Extract a class definition.
fn extract_class(
    result: &mut ParseResult,
    scope_stack: &mut Vec<usize>,
    content: &[u8],
    path: &Path,
    file_stem: &str,
    node: Node,
    decorators: &[String],
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
            visit_node(result, scope_stack, content, path, file_stem, child, &[]);
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
/// references), not the original module name.
fn extract_import(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "dotted_name" || child.kind() == "aliased_import" {
            let (import_name, symbol_name) = if child.kind() == "aliased_import" {
                let original = child
                    .child_by_field_name("name")
                    .map(|n| node_text(content, n).to_string())
                    .unwrap_or_default();
                let alias = child
                    .child_by_field_name("alias")
                    .map(|n| node_text(content, n).to_string());
                // Symbol name is the alias (what code uses), falling back to original
                let sym_name = alias.unwrap_or_else(|| original.clone());
                (original, sym_name)
            } else {
                let name = node_text(content, child).to_string();
                (name.clone(), name)
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
                    annotations: vec!["import".to_string()],
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
fn extract_import_from(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
    let file_stem = path
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| "unknown".to_string());

    let full_text = node_text(content, node);
    let is_relative = full_text.contains("from .") || full_text.starts_with("from .");
    let is_star = full_text.contains("import *");

    if is_star {
        result.references.push(Reference {
            id: ReferenceId::default(),
            from: SymbolId::default(),
            to: SymbolId::default(),
            kind: ReferenceKind::Import,
            location: node_location(path, node),
            resolution: ResolutionStatus::Partial("star import".to_string()),
        });
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
                        annotations: vec!["import".to_string()],
                    });

                    found_names = true;
                }
            }
            "aliased_import" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = node_text(content, name_node).to_string();
                    if !name.is_empty() {
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
                        let symbol_name = child
                            .child_by_field_name("alias")
                            .map(|n| node_text(content, n).to_string())
                            .unwrap_or_else(|| name.clone());

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
                            annotations: vec!["import".to_string()],
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

/// Recursively walks the AST to extract all function/method call references.
///
/// Creates a `Reference` with `kind: ReferenceKind::Call` for each `call` node found.
/// The callee name is stored in `resolution: ResolutionStatus::Partial("call:<name>")`
/// for later resolution by `populate_graph`. Both `from` and `to` are left as
/// `SymbolId::default()` — `populate_graph` resolves them via location containment
/// and name matching respectively.
fn extract_all_calls(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
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
        extract_all_calls(result, content, path, child);
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
fn extract_attribute_accesses(result: &mut ParseResult, content: &[u8], path: &Path, node: Node) {
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
        extract_attribute_accesses(result, content, path, child);
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
}
