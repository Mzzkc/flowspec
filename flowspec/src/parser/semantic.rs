//! Source-derived semantic prerequisites. These are separate from compatibility
//! entities: every binding is lexical, and every absence proof names its scope.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tree_sitter::Node;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticValue {
    pub name: String,
    pub owner: String,
    pub scope: usize,
    pub ordinal: usize,
    pub range: (usize, usize),
    pub initializer: (usize, usize),
    pub produced_type: String,
    pub protocol: String,
    pub reads: Vec<(usize, usize)>,
    pub writes: Vec<(usize, usize)>,
    pub closure: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticIssue {
    pub pattern: String,
    pub reason: String,
    pub range: (usize, usize),
    pub operation: String,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SemanticFile {
    pub path: PathBuf,
    pub values: Vec<SemanticValue>,
    pub issues: Vec<SemanticIssue>,
    #[serde(default)]
    pub functions: Vec<super::normalization::NormalizedFunction>,
    #[serde(default)]
    pub executables: Vec<ExecutableBinding>,
    #[serde(default)]
    pub import_uses: Vec<ImportUse>,
    #[serde(default)]
    pub normal_control: Vec<super::control::NormalControl>,
    #[serde(default)]
    pub signatures: Vec<super::contracts::SignatureFact>,
    #[serde(default)]
    pub literal_calls: Vec<super::contracts::LiteralCall>,
    #[serde(default)]
    pub private_implementations: Vec<super::private_impl::PrivateImplementation>,
}

fn text<'a>(source: &'a str, node: Node<'_>) -> &'a str {
    &source[node.byte_range()]
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).collect()
}
fn function(node: Node<'_>) -> bool {
    matches!(
        node.kind(),
        "function_declaration"
            | "function_definition"
            | "function_item"
            | "arrow_function"
            | "function_expression"
            | "method_definition"
    )
}
fn range(node: Node<'_>) -> (usize, usize) {
    (node.start_byte(), node.end_byte())
}
#[derive(Clone)]
struct Binder {
    name: String,
    scope: usize,
    initialized_at: usize,
    value: Option<usize>,
    import_binding: bool,
}
struct Scope {
    parent: Option<usize>,
}
struct Collector<'a> {
    source: &'a str,
    file: SemanticFile,
    scopes: Vec<Scope>,
    binders: Vec<Binder>,
    node_scopes: std::collections::HashMap<usize, usize>,
    declarations: std::collections::HashSet<usize>,
}
impl Collector<'_> {
    fn lookup(&self, name: &str, mut scope: usize) -> Option<&Binder> {
        loop {
            let mut found = self
                .binders
                .iter()
                .filter(|b| b.scope == scope && b.name == name);
            if let Some(b) = found.next() {
                // Repeated declaration is ambiguous until redeclaration/hoisting
                // semantics have been modelled. Never pick a convenient one.
                return found.next().is_none().then_some(b);
            }
            scope = self.scopes[scope].parent?;
        }
    }
    fn collect(&mut self, node: Node<'_>, mut scope: usize, owner: &str, depth: usize) {
        if depth > super::MAX_AST_DEPTH {
            self.issue(node, "depth_truncation", "lexical traversal");
            return;
        }
        let mut owner = owner.to_string();
        if function(node) || matches!(node.kind(), "statement_block" | "block") {
            let id = self.scopes.len();
            self.scopes.push(Scope {
                parent: Some(scope),
            });
            scope = id;
            if function(node) {
                if let Some(name) = node.child_by_field_name("name") {
                    owner = if owner.is_empty() {
                        text(self.source, name).to_string()
                    } else {
                        format!("{owner}::{}", text(self.source, name))
                    };
                    self.declarations.insert(name.start_byte());
                }
                if let Some(params) = node.child_by_field_name("parameters") {
                    for param in children(params) {
                        let id = if param.kind() == "identifier" {
                            Some(param)
                        } else {
                            param
                                .child_by_field_name("pattern")
                                .or_else(|| param.child_by_field_name("name"))
                        };
                        if let Some(id) = id.filter(|id| id.kind() == "identifier") {
                            self.declarations.insert(id.start_byte());
                            self.binders.push(Binder {
                                name: text(self.source, id).to_string(),
                                scope,
                                initialized_at: node.start_byte(),
                                value: None,
                                import_binding: false,
                            });
                        }
                    }
                }
            }
        }
        if node.kind() == "import_statement" {
            fn imports<'a>(node: Node<'a>, out: &mut Vec<Node<'a>>) {
                if node.kind() == "import_specifier" {
                    if let Some(name) = node
                        .child_by_field_name("alias")
                        .or_else(|| node.child_by_field_name("name"))
                    {
                        out.push(name);
                    }
                    return;
                }
                if node.kind() == "namespace_import" {
                    if let Some(name) = children(node).last() {
                        out.push(*name);
                    }
                    return;
                }
                for child in children(node) {
                    if node.kind() == "import_clause" && child.kind() == "identifier" {
                        out.push(child);
                    } else {
                        imports(child, out);
                    }
                }
            }
            let mut names = Vec::new();
            imports(node, &mut names);
            for name in names {
                self.binders.push(Binder {
                    name: text(self.source, name).to_string(),
                    scope,
                    initialized_at: 0,
                    value: None,
                    import_binding: true,
                });
            }
            let mut pending = vec![node];
            while let Some(n) = pending.pop() {
                if n.kind() == "identifier" {
                    self.declarations.insert(n.start_byte());
                }
                pending.extend(children(n));
            }
        }
        self.node_scopes.insert(node.start_byte(), scope);
        if node.kind() == "variable_declarator" && !owner.is_empty() {
            if let (Some(name), Some(init)) = (
                node.child_by_field_name("name"),
                node.child_by_field_name("value"),
            ) {
                if name.kind() == "identifier" {
                    self.declarations.insert(name.start_byte());
                    let declaration_kind = node
                        .parent()
                        .and_then(|p| p.child(0))
                        .map(|n| text(self.source, n))
                        .unwrap_or("");
                    let is_lexical = matches!(declaration_kind, "const" | "let");
                    let value = self.file.values.len();
                    self.file.values.push(SemanticValue {
                        name: text(self.source, name).to_string(),
                        owner: owner.clone(),
                        scope,
                        ordinal: self.binders.iter().filter(|b| b.scope == scope).count(),
                        range: range(node),
                        initializer: range(init),
                        produced_type: String::new(),
                        protocol: String::new(),
                        reads: Vec::new(),
                        writes: Vec::new(),
                        closure: is_lexical,
                    });
                    self.binders.push(Binder {
                        name: text(self.source, name).to_string(),
                        scope,
                        initialized_at: init.end_byte(),
                        value: Some(value),
                        import_binding: false,
                    });
                    if !is_lexical {
                        self.issue(
                            node,
                            "value_binding_unresolved",
                            "var hoisting and redeclaration",
                        );
                    }
                } else {
                    self.issue(name, "value_binding_unresolved", "destructuring binding");
                }
            }
        }
        for child in children(node) {
            self.collect(child, scope, &owner, depth + 1);
        }
    }
    fn issue(&mut self, node: Node<'_>, reason: &str, operation: &str) {
        self.file.issues.push(SemanticIssue {
            pattern: "data_dead_end".into(),
            reason: reason.into(),
            range: range(node),
            operation: operation.into(),
        });
    }
    fn uses(&mut self, node: Node<'_>, scope: usize, depth: usize) {
        if depth > super::MAX_AST_DEPTH {
            return;
        }
        let scope = self
            .node_scopes
            .get(&node.start_byte())
            .copied()
            .unwrap_or(scope);
        if matches!(
            node.kind(),
            "identifier" | "shorthand_property_identifier" | "type_identifier"
        ) && !self.declarations.contains(&node.start_byte())
        {
            let name = text(self.source, node);
            if let Some(b) = self.lookup(name, scope).cloned() {
                if b.import_binding
                    && !node.parent().is_some_and(|p| {
                        p.kind() == "export_specifier"
                            && p.child_by_field_name("alias") == Some(node)
                    })
                {
                    let kind = if node.kind() == "type_identifier" {
                        super::ir::ReferenceKind::Annotation
                    } else if node
                        .parent()
                        .is_some_and(|p| p.kind() == "export_specifier")
                    {
                        super::ir::ReferenceKind::Export
                    } else {
                        super::ir::ReferenceKind::Read
                    };
                    self.file.import_uses.push(ImportUse {
                        name: name.to_string(),
                        range: range(node),
                        kind,
                    });
                }
                if let Some(value) = b.value {
                    let initialized_at = b.initialized_at;
                    let is_write = node.parent().is_some_and(|p| {
                        p.kind() == "assignment_expression"
                            && p.child_by_field_name("left") == Some(node)
                    });
                    if is_write {
                        self.file.values[value].writes.push(range(node));
                    } else {
                        self.file.values[value].reads.push(range(node));
                    }
                    if node.start_byte() < initialized_at {
                        self.file.values[value].closure = false;
                    }
                }
            }
        }
        if matches!(node.kind(), "with_statement")
            || (node.kind() == "identifier"
                && matches!(text(self.source, node), "eval" | "Function"))
        {
            // Direct or indirect eval/function construction can observe bindings.
            // A local shadow is still conservative until its invocation is proved.
            self.issue(node, "value_escape_unresolved", "reflective lexical access");
            for b in &self.binders {
                if let Some(v) = b.value {
                    let mut s = scope;
                    loop {
                        if s == b.scope {
                            self.file.values[v].closure = false;
                            break;
                        }
                        let Some(parent) = self.scopes[s].parent else {
                            break;
                        };
                        s = parent;
                    }
                }
            }
        }
        for child in children(node) {
            self.uses(child, scope, depth + 1);
        }
    }
    fn primitive(&self, node: Node<'_>, scope: usize) -> bool {
        match node.kind() {
            "null" | "true" | "false" | "number" | "string" => true,
            "identifier" => self
                .lookup(text(self.source, node), scope)
                .is_some_and(|b| b.initialized_at <= node.start_byte()),
            "parenthesized_expression" => children(node)
                .first()
                .is_some_and(|n| self.primitive(*n, scope)),
            _ => false,
        }
    }
    fn productions(&mut self, node: Node<'_>, depth: usize) {
        if depth > super::MAX_AST_DEPTH {
            return;
        }
        if node.kind() == "variable_declarator" {
            if let Some(init) = node.child_by_field_name("value") {
                let scope = self
                    .node_scopes
                    .get(&node.start_byte())
                    .copied()
                    .unwrap_or(0);
                let strict = init.kind() == "binary_expression"
                    && init
                        .child_by_field_name("operator")
                        .is_some_and(|op| matches!(text(self.source, op), "===" | "!=="))
                    && init
                        .child_by_field_name("left")
                        .is_some_and(|n| self.primitive(n, scope))
                    && init
                        .child_by_field_name("right")
                        .is_some_and(|n| self.primitive(n, scope));
                let literal =
                    matches!(init.kind(), "true" | "false" | "null" | "number" | "string");
                if let Some(value) = self.file.values.iter_mut().find(|v| v.range == range(node)) {
                    if strict {
                        value.produced_type = "boolean".into();
                        value.protocol = "ECMA-262.StrictEqualityComparison".into();
                    } else if literal {
                        value.produced_type = init.kind().into();
                        value.protocol = "ECMA-262.LiteralEvaluation".into();
                    } else {
                        value.closure = false;
                        self.issue(
                            init,
                            "value_effect_unresolved",
                            "initializer result/effect protocol",
                        );
                    }
                }
            }
        }
        for child in children(node) {
            self.productions(child, depth + 1);
        }
    }
}
/// Derive facts only from the original bytes and the registry's actual grammar.
/// Transformed TS input is a located missing prerequisite, never complete proof.
pub(crate) fn collect(path: &Path, source: &str) -> SemanticFile {
    let empty = || SemanticFile {
        path: path.to_path_buf(),
        ..Default::default()
    };
    let Some((language, parsed_source)) = super::registry_grammar_source(path, source) else {
        return empty();
    };
    let mut parser = tree_sitter::Parser::new();
    if parser.set_language(&language).is_err() {
        return empty();
    }
    let Some(tree) = parser.parse(source, None) else {
        return empty();
    };
    let mut collector = Collector {
        source,
        file: empty(),
        scopes: vec![Scope { parent: None }],
        binders: Vec::new(),
        node_scopes: Default::default(),
        declarations: Default::default(),
    };
    let root = tree.root_node();
    if super::has_parse_loss(root) || parsed_source != source {
        collector.issue(
            root,
            "value_binding_unresolved",
            "original-byte grammar/recovery",
        );
        return collector.file;
    }
    collector.file.normal_control = super::control::collect(source, root);
    (collector.file.signatures, collector.file.literal_calls) =
        super::contracts::collect(source, root);
    let language_id = crate::language_metadata::language_id_for_path(path);
    let mut pending = vec![(root, 0usize)];
    while let Some((node, depth)) = pending.pop() {
        if depth > super::MAX_AST_DEPTH {
            collector.issue(node, "depth_truncation", "source prerequisite enumeration");
            continue;
        }
        if (language_id == Some("python") && node.kind() == "assignment")
            || (language_id == Some("rust") && node.kind() == "let_declaration")
        {
            collector.issue(node,"value_binding_unresolved","local produced-value binding/effect/consumer closure not implemented for this grammar");
        }
        if language_id == Some("rust")
            && matches!(
                node.kind(),
                "macro_invocation" | "attribute_item" | "inner_attribute_item"
            )
        {
            collector.issue(
                node,
                "registration_closure_unresolved",
                "macro/attribute executable registration",
            );
        }
        for child in children(node) {
            pending.push((child, depth + 1));
        }
    }
    if crate::language_metadata::language_id_for_path(path) == Some("rust") {
        collector.file.executables = collect_rust_executables(source, path, root);
        collector.file.private_implementations = super::private_impl::collect(
            source,
            &path.file_name().unwrap_or_default().to_string_lossy(),
            root,
        );
    }
    if let Some(language) = crate::language_metadata::language_id_for_path(path) {
        collector.file.functions = super::normalization::collect(source, language, root);
    }
    // Value protocols currently cover ECMAScript lexical declarations only.
    // Other languages are handled by their executable graph prerequisites.
    if matches!(
        crate::language_metadata::language_id_for_path(path),
        Some("javascript" | "typescript")
    ) {
        collector.collect(root, 0, "", 0);
        collector.uses(root, 0, 0);
        collector.productions(root, 0);
    }
    collector.file
}

/// Resolved standalone executable closure. Trait/impl/attribute/macro routes
/// are explicitly outside this proof, never inferred from an unread spelling.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutableBinding {
    pub qualified_name: String,
    pub range: (usize, usize),
    pub externally_visible: bool,
    pub consumers: Vec<(usize, usize)>,
    pub closure: bool,
}
fn collect_rust_executables(source: &str, path: &Path, root: Node<'_>) -> Vec<ExecutableBinding> {
    fn qualified(file: &str, modules: &[String], name: &str) -> String {
        let mut parts = vec![file.to_string()];
        parts.extend_from_slice(modules);
        parts.push(name.to_string());
        parts.join("::")
    }
    let file = path.file_name().unwrap_or_default().to_string_lossy();
    let mut bindings = Vec::new();
    let mut uses = Vec::new();
    let mut declarations = std::collections::HashSet::new();
    let mut pending = vec![(root, Vec::<String>::new(), true, false, 0usize)];
    let mut closed = true;
    while let Some((node, mut modules, mut visible, in_impl, depth)) = pending.pop() {
        if depth > super::MAX_AST_DEPTH {
            closed = false;
            continue;
        }
        let public = children(node)
            .iter()
            .find(|n| n.kind() == "visibility_modifier")
            .is_some_and(|n| text(source, *n) == "pub");
        if node.kind() == "mod_item" {
            visible &= public;
            if let Some(name) = node.child_by_field_name("name") {
                modules.push(text(source, name).to_string());
                declarations.insert(name.start_byte());
            }
        }
        if matches!(
            node.kind(),
            "macro_invocation" | "attribute_item" | "inner_attribute_item"
        ) {
            closed = false;
        }
        if node.kind() == "function_item" {
            if let Some(name) = node.child_by_field_name("name") {
                declarations.insert(name.start_byte());
                if !in_impl {
                    bindings.push(ExecutableBinding {
                        qualified_name: qualified(&file, &modules, text(source, name)),
                        range: range(node),
                        externally_visible: visible && public,
                        consumers: Vec::new(),
                        closure: true,
                    });
                }
            }
            if let Some(params) = node.child_by_field_name("parameters") {
                for parameter in children(params) {
                    if let Some(name) = parameter.child_by_field_name("pattern") {
                        declarations.insert(name.start_byte());
                    }
                }
            }
        }
        if node.kind() == "let_declaration" {
            if let Some(name) = node.child_by_field_name("pattern") {
                declarations.insert(name.start_byte());
            }
        }
        if node.kind() == "scoped_identifier" {
            let raw = text(source, node);
            let mut parts: Vec<String> = raw.split("::").map(str::to_string).collect();
            let resolved = if parts.first().is_some_and(|p| p == "crate") {
                parts.remove(0);
                parts
            } else if parts.first().is_some_and(|p| p == "self") {
                parts.remove(0);
                let mut m = modules.clone();
                m.extend(parts);
                m
            } else if parts.first().is_some_and(|p| p == "super") {
                parts.remove(0);
                let mut m = modules.clone();
                m.pop();
                m.extend(parts);
                m
            } else {
                let mut m = modules.clone();
                m.extend(parts);
                m
            };
            uses.push((format!("{}::{}", file, resolved.join("::")), range(node)));
        } else if node.kind() == "identifier"
            && node
                .parent()
                .is_none_or(|p| p.kind() != "scoped_identifier")
        {
            uses.push((qualified(&file, &modules, text(source, node)), range(node)));
        }
        let mut next = children(node);
        next.reverse();
        for child in next {
            pending.push((
                child,
                modules.clone(),
                visible,
                in_impl || matches!(node.kind(), "impl_item" | "trait_item"),
                depth + 1,
            ));
        }
    }
    // A same-spelling local may shadow a standalone item. Until its lexical
    // resolution is available, treat any such use as a consumer (safe refusal).
    for binding in &mut bindings {
        binding.closure = closed;
        binding.consumers = uses
            .iter()
            .filter(|(name, r)| name == &binding.qualified_name && !declarations.contains(&r.0))
            .map(|(_, r)| *r)
            .collect();
    }
    bindings
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportUse {
    pub name: String,
    pub range: (usize, usize),
    pub kind: super::ir::ReferenceKind,
}
