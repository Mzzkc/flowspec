//! Typed, source-owned module dependency, migration and public-surface proofs.
//! This module does not own private implementation or peer control-flow policy.
use crate::config::Config;
use crate::graph::Graph;
use crate::parser::ir::{ReferenceKind, ResolutionStatus, Symbol, SymbolId};
use crate::report_authority::{
    digest, hash, location_range, loss, offset_range, path_key, InputRecord,
};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use tree_sitter::Node;

const OWNED: [&str; 3] = [
    "layer_violation",
    "incomplete_migration",
    "missing_reexport",
];
fn binding(graph: &Graph, root: &Path, locator: &str) -> Option<SymbolId> {
    let (path, name) = locator.split_once("::")?;
    let path = root.join(path);
    let stem = path.file_stem()?.to_str()?;
    let filename = path.file_name()?.to_str()?;
    let mut found = graph.all_symbols().filter(|(_, s)| {
        s.location.file == path
            && !s.annotations.iter().any(|a| a == "import")
            && (s.qualified_name == format!("{stem}::{name}")
                || s.qualified_name == format!("{filename}::{name}"))
    });
    match (found.next(), found.next()) {
        (Some((id, _)), None) => Some(id),
        _ => None,
    }
}
fn base(root: &Path, input: &InputRecord, s: &Symbol) -> Value {
    json!({"path_key":path_key(root,&input.path),"language":input.language,"qualified_name":s.qualified_name,"construct_kind":format!("{:?}",s.kind).to_lowercase()})
}
fn debt(
    pattern: &str,
    root: &Path,
    input: Option<&InputRecord>,
    reason: &str,
    policy: Value,
    losses: &mut Vec<Value>,
) {
    let mut item = loss(
        reason,
        input.map(|i| path_key(root, &i.path)).unwrap_or_default(),
        None,
        "module_policy",
    );
    item["affected_patterns"] = json!([pattern]);
    item["policy"] = policy;
    item["id"] = json!(hash("module-policy.loss", &item));
    losses.push(item);
}
fn finding(
    pattern: &str,
    subject: Value,
    range: Value,
    content: &str,
    policy: Value,
    relation: &str,
    evidence: Value,
) -> Value {
    let cause = json!({"key_version":1,"parts":{"relation":relation,"policy":policy}});
    let mut identity_subject = subject.clone();
    identity_subject.as_object_mut().unwrap().remove("name");
    let semantic_id = hash(
        "finding.semantic",
        &json!({"pattern_id":pattern,"subject":identity_subject,"cause":cause}),
    );
    let evidence = json!([evidence]);
    let observation_id = hash(
        "finding.observation",
        &json!({"semantic_id":semantic_id,"input":digest(content.as_bytes()),"range":range,"evidence":evidence}),
    );
    json!({"identity_status":"stable","semantic_id":semantic_id,"observation_id":observation_id,"pattern_id":pattern,"severity":"warning","confidence":"high","subject":subject,"cause":cause,"range":range,"evidence":evidence,"suggestion":"Reconcile this exact source binding with the declared policy."})
}
fn input<'a>(inputs: &'a [InputRecord], path: &Path) -> Option<&'a InputRecord> {
    inputs.iter().find(|i| {
        i.path == path
            && i.disposition == "selected"
            && i.parsed
            && !i.changed_during_run
            && i.content.is_some()
    })
}
fn normalize(path: PathBuf) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                if !out.pop() {
                    return None;
                }
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    Some(out)
}
fn module_path(
    root: &Path,
    inputs: &[InputRecord],
    origin: &InputRecord,
    name: &str,
) -> Option<PathBuf> {
    let language = origin.language.as_deref()?;
    let base = match language {
        "python" => {
            let dots = name.chars().take_while(|c| *c == '.').count();
            let mut parent = if dots == 0 {
                root.to_path_buf()
            } else {
                origin.path.parent()?.to_path_buf()
            };
            for _ in 1..dots {
                parent = parent.parent()?.to_path_buf();
            }
            parent.join(name[dots..].replace('.', "/"))
        }
        "javascript" | "typescript" => {
            if !name.starts_with('.') {
                return None;
            }
            origin.path.parent()?.join(name)
        }
        "rust" => {
            let mut parts = name.split("::");
            let first = parts.next()?;
            let parent = match first {
                "crate" => root.to_path_buf(),
                "self" => origin.path.parent()?.to_path_buf(),
                "super" => origin.path.parent()?.parent()?.to_path_buf(),
                _ => origin.path.parent()?.to_path_buf(),
            };
            let rest = if matches!(first, "crate" | "self" | "super") {
                parts.collect::<Vec<_>>().join("/")
            } else {
                name.replace("::", "/")
            };
            parent.join(rest)
        }
        _ => return None,
    };
    let base = normalize(base)?;
    let extensions: &[&str] = match language {
        "python" => &["py"],
        "rust" => &["rs"],
        "typescript" => &["ts", "tsx", "js", "jsx"],
        _ => &["js", "jsx"],
    };
    let mut candidates = vec![base.clone()];
    for ext in extensions {
        candidates.push(base.with_extension(ext));
    }
    match language {
        "python" => candidates.push(base.join("__init__.py")),
        "rust" => candidates.push(base.join("mod.rs")),
        _ => {
            for ext in extensions {
                candidates.push(base.join(format!("index.{ext}")));
            }
        }
    }
    let selected: BTreeSet<_> = candidates
        .into_iter()
        .filter(|p| p.starts_with(root) && input(inputs, p).is_some())
        .collect();
    (selected.len() == 1).then(|| selected.into_iter().next().unwrap())
}
fn children(node: Node<'_>) -> Vec<Node<'_>> {
    let mut c = node.walk();
    node.named_children(&mut c).collect()
}
fn text<'a>(source: &'a str, node: Node<'_>) -> &'a str {
    &source[node.byte_range()]
}
fn string(source: &str, node: Node<'_>) -> Option<String> {
    let t = text(source, node);
    if t.len() < 2 {
        return None;
    }
    let quote = t.as_bytes()[0];
    if !matches!(quote, b'\'' | b'"')
        || t.as_bytes()[t.len() - 1] != quote
        || t[1..t.len() - 1].contains(['\\', '\n'])
    {
        return None;
    }
    Some(t[1..t.len() - 1].into())
}
#[derive(Default)]
struct Surface {
    exports: BTreeMap<String, SymbolId>,
    dependencies: Vec<(PathBuf, (usize, usize))>,
}
struct Exports<'a> {
    root: &'a Path,
    inputs: &'a [InputRecord],
    graph: &'a Graph,
    active: BTreeSet<PathBuf>,
}
impl Exports<'_> {
    fn local(&self, file: &InputRecord, name: &str) -> Option<SymbolId> {
        let relative = file.path.strip_prefix(self.root).ok()?.to_str()?;
        binding(self.graph, self.root, &format!("{relative}::{name}"))
    }
    fn remote(
        &mut self,
        file: &InputRecord,
        module: &str,
    ) -> Result<(PathBuf, Surface), &'static str> {
        let path =
            module_path(self.root, self.inputs, file, module).ok_or("policy_module_unresolved")?;
        let surface = self.surface(&path)?;
        Ok((path, surface))
    }
    fn surface(&mut self, path: &Path) -> Result<Surface, &'static str> {
        if self.active.len() > 64 || !self.active.insert(path.to_path_buf()) {
            return Err("policy_export_cycle_unresolved");
        }
        let result = self.parse_surface(path);
        self.active.remove(path);
        result
    }
    fn parse_surface(&mut self, path: &Path) -> Result<Surface, &'static str> {
        let file = input(self.inputs, path).ok_or("policy_module_unresolved")?;
        if self.graph.has_dynamic_export(path) {
            return Err("policy_export_unresolved");
        }
        let source = file.content.as_deref().unwrap();
        let lang = file
            .language
            .as_deref()
            .ok_or("policy_language_unresolved")?;
        let grammar = match lang {
            "python" => tree_sitter_python::LANGUAGE.into(),
            "javascript" => tree_sitter_javascript::LANGUAGE.into(),
            "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            "rust" => tree_sitter_rust::LANGUAGE.into(),
            _ => return Err("policy_language_unresolved"),
        };
        let mut parser = tree_sitter::Parser::new();
        parser
            .set_language(&grammar)
            .map_err(|_| "policy_grammar_unresolved")?;
        let tree = parser
            .parse(source, None)
            .ok_or("policy_parse_unresolved")?;
        if tree.root_node().has_error() {
            return Err("policy_parse_unresolved");
        }
        let mut surface = Surface::default();
        let mut locals = BTreeMap::new();
        let mut all = None;
        for node in children(tree.root_node()) {
            let declaration = node.child_by_field_name("declaration").unwrap_or(node);
            if let Some(name) = declaration.child_by_field_name("name") {
                let name = text(source, name);
                if let Some(id) = self.local(file, name) {
                    locals.insert(name.to_string(), id);
                    let public = match lang {
                        "python" => !name.starts_with('_'),
                        "rust" => children(node).iter().any(|n| {
                            n.kind() == "visibility_modifier" && text(source, *n) == "pub"
                        }),
                        _ => node.kind() == "export_statement",
                    };
                    if public {
                        surface.exports.insert(name.into(), id);
                    }
                }
            }
            match (lang, node.kind()) {
                ("python", "import_from_statement") => {
                    let module = node
                        .child_by_field_name("module_name")
                        .ok_or("policy_export_unresolved")?;
                    let (target, remote) = self.remote(file, text(source, module))?;
                    surface
                        .dependencies
                        .push((target, (node.start_byte(), node.end_byte())));
                    for n in children(node) {
                        match n.kind() {
                            "wildcard_import" => {
                                for (name, id) in remote.exports {
                                    locals.insert(name.clone(), id);
                                    surface.exports.insert(name, id);
                                }
                                break;
                            }
                            "dotted_name" | "aliased_import" if n.id() != module.id() => {
                                let original = n.child_by_field_name("name").unwrap_or(n);
                                let alias = n.child_by_field_name("alias").unwrap_or(original);
                                let id = *remote
                                    .exports
                                    .get(text(source, original))
                                    .ok_or("policy_export_binding_unresolved")?;
                                let name = text(source, alias).to_string();
                                locals.insert(name.clone(), id);
                                if !name.starts_with('_') {
                                    surface.exports.insert(name, id);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                ("python", "expression_statement") => {
                    for n in children(node) {
                        if n.kind() == "assignment"
                            && n.child_by_field_name("left")
                                .is_some_and(|n| text(source, n) == "__all__")
                        {
                            if all.is_some() {
                                return Err("policy_export_unresolved");
                            }
                            let right = n
                                .child_by_field_name("right")
                                .ok_or("policy_export_unresolved")?;
                            if !matches!(right.kind(), "list" | "tuple") {
                                return Err("policy_export_unresolved");
                            }
                            let names: Option<Vec<_>> = children(right)
                                .into_iter()
                                .map(|n| string(source, n))
                                .collect();
                            all = Some(names.ok_or("policy_export_unresolved")?);
                        }
                    }
                }
                ("javascript" | "typescript", "export_statement") => {
                    let remote = if let Some(module) = node.child_by_field_name("source") {
                        let name = string(source, module).ok_or("policy_export_unresolved")?;
                        let (target, s) = self.remote(file, &name)?;
                        surface
                            .dependencies
                            .push((target, (node.start_byte(), node.end_byte())));
                        Some(s)
                    } else {
                        None
                    };
                    let clause = children(node)
                        .into_iter()
                        .find(|n| n.kind() == "export_clause");
                    if let Some(clause) = clause {
                        for spec in children(clause) {
                            let name = spec
                                .child_by_field_name("name")
                                .ok_or("policy_export_unresolved")?;
                            let alias = spec.child_by_field_name("alias").unwrap_or(name);
                            let id = if let Some(remote) = &remote {
                                *remote
                                    .exports
                                    .get(text(source, name))
                                    .ok_or("policy_export_binding_unresolved")?
                            } else {
                                *locals
                                    .get(text(source, name))
                                    .or_else(|| surface.exports.get(text(source, name)))
                                    .ok_or("policy_export_binding_unresolved")?
                            };
                            surface.exports.insert(text(source, alias).into(), id);
                        }
                    } else if let Some(remote) = remote {
                        if children(node)
                            .iter()
                            .any(|n| n.kind() == "namespace_export")
                        {
                            return Err("policy_export_unresolved");
                        }
                        surface
                            .exports
                            .extend(remote.exports.into_iter().filter(|(n, _)| n != "default"));
                    } else if node.child_by_field_name("value").is_some() {
                        return Err("policy_export_unresolved");
                    }
                }
                ("rust", "use_declaration") => {
                    let argument = node
                        .child_by_field_name("argument")
                        .ok_or("policy_export_unresolved")?;
                    let public = children(node)
                        .iter()
                        .any(|n| n.kind() == "visibility_modifier" && text(source, *n) == "pub");
                    self.rust_use(file, argument, source, "", public, &mut surface)?;
                }
                _ => {}
            }
        }
        if let Some(all) = all {
            surface.exports.clear();
            for name in all {
                let id = *locals
                    .get(&name)
                    .ok_or("policy_export_binding_unresolved")?;
                surface.exports.insert(name, id);
            }
        }
        Ok(surface)
    }
    fn rust_use(
        &mut self,
        file: &InputRecord,
        node: Node<'_>,
        source: &str,
        prefix: &str,
        public: bool,
        surface: &mut Surface,
    ) -> Result<(), &'static str> {
        if node.kind() == "scoped_use_list" {
            let path = node
                .child_by_field_name("path")
                .map(|n| text(source, n))
                .unwrap_or("");
            let prefix = if prefix.is_empty() {
                path.to_string()
            } else {
                format!("{prefix}::{path}")
            };
            let list = node
                .child_by_field_name("list")
                .ok_or("policy_export_unresolved")?;
            for child in children(list) {
                self.rust_use(file, child, source, &prefix, public, surface)?;
            }
            return Ok(());
        }
        let original = node
            .child_by_field_name("path")
            .filter(|_| node.kind() == "use_as_clause")
            .unwrap_or(node);
        let raw = text(source, original);
        let raw = if prefix.is_empty() {
            raw.to_string()
        } else {
            format!("{prefix}::{raw}")
        };
        let (module, name) = raw.rsplit_once("::").ok_or("policy_module_unresolved")?;
        let (target, remote) = self.remote(file, module)?;
        surface
            .dependencies
            .push((target, (node.start_byte(), node.end_byte())));
        if name == "*" {
            if public {
                surface.exports.extend(remote.exports);
            }
        } else {
            let id = *remote
                .exports
                .get(name)
                .ok_or("policy_export_binding_unresolved")?;
            let alias = node
                .child_by_field_name("alias")
                .map(|n| text(source, n))
                .unwrap_or(name);
            if public {
                surface.exports.insert(alias.into(), id);
            }
        }
        Ok(())
    }
}

fn grammar(language: &str) -> Option<tree_sitter::Language> {
    Some(match language {
        "python" => tree_sitter_python::LANGUAGE.into(),
        "javascript" => tree_sitter_javascript::LANGUAGE.into(),
        "typescript" => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
        "rust" => tree_sitter_rust::LANGUAGE.into(),
        _ => return None,
    })
}
fn shadowed(call: Node<'_>, source: &str, name: &str) -> bool {
    fn parameter_has(node: Node<'_>, source: &str, name: &str) -> bool {
        (node.kind() == "identifier" && text(source, node) == name)
            || children(node)
                .into_iter()
                .any(|n| parameter_has(n, source, name))
    }
    fn local_has(node: Node<'_>, source: &str, name: &str) -> bool {
        if matches!(
            node.kind(),
            "function_definition"
                | "function_declaration"
                | "function_item"
                | "class_definition"
                | "class_declaration"
        ) {
            return node
                .child_by_field_name("name")
                .is_some_and(|n| text(source, n) == name);
        }
        let target = match node.kind() {
            "assignment" | "augmented_assignment" => node.child_by_field_name("left"),
            "variable_declarator" => node.child_by_field_name("name"),
            "let_declaration" => node.child_by_field_name("pattern"),
            _ => None,
        };
        target.is_some_and(|n| parameter_has(n, source, name))
            || children(node)
                .into_iter()
                .any(|n| local_has(n, source, name))
    }
    let mut enclosing = call.parent();
    while let Some(node) = enclosing {
        if matches!(
            node.kind(),
            "function_definition"
                | "function_declaration"
                | "function_item"
                | "arrow_function"
                | "function_expression"
                | "method_definition"
        ) && (node
            .child_by_field_name("parameters")
            .is_some_and(|n| parameter_has(n, source, name))
            || node
                .child_by_field_name("body")
                .is_some_and(|n| local_has(n, source, name)))
        {
            return true;
        }
        enclosing = node.parent();
    }
    false
}
fn live_calls(
    root: &Path,
    inputs: &[InputRecord],
    graph: &Graph,
    authority: SymbolId,
) -> Vec<Value> {
    let target = graph.get_symbol(authority).unwrap();
    let mut consumers = Vec::new();
    for file in inputs
        .iter()
        .filter(|i| i.disposition == "selected" && i.parsed && !i.changed_during_run)
    {
        let Some(source) = file.content.as_deref() else {
            continue;
        };
        let Some(grammar) = file.language.as_deref().and_then(grammar) else {
            continue;
        };
        let mut parser = tree_sitter::Parser::new();
        if parser.set_language(&grammar).is_err() {
            continue;
        }
        let Some(tree) = parser.parse(source, None) else {
            continue;
        };
        if tree.root_node().has_error() {
            continue;
        }
        let mut names = BTreeSet::new();
        if target.location.file == file.path {
            names.insert(target.name.clone());
        }
        for (id, symbol) in graph.all_symbols().filter(|(_, s)| {
            s.location.file == file.path && s.annotations.iter().any(|a| a == "import")
        }) {
            let targets: BTreeSet<_> = graph
                .edges_from(id)
                .iter()
                .filter(|e| {
                    e.kind == crate::parser::ir::EdgeKind::References
                        && e.target != id
                        && e.reference_id
                            .and_then(|rid| graph.get_reference(rid))
                            .is_some_and(|r| {
                                r.kind == ReferenceKind::Import
                                    && r.resolution == ResolutionStatus::Resolved
                            })
                })
                .map(|e| e.target)
                .collect();
            if targets.len() == 1 && targets.contains(&authority) {
                names.insert(symbol.name.clone());
            }
        }
        // Resolve named imports from their actual grammar-owned origin,
        // independently of graph compatibility import adjacency.
        for n in children(tree.root_node()) {
            let mut imports = Vec::new();
            match (file.language.as_deref().unwrap_or(""), n.kind()) {
                ("python", "import_from_statement") => {
                    if let Some(module) = n.child_by_field_name("module_name") {
                        if let Some(path) = module_path(root, inputs, file, text(source, module)) {
                            for child in children(n) {
                                if child.id() == module.id() {
                                    continue;
                                }
                                if matches!(child.kind(), "aliased_import" | "dotted_name") {
                                    let original =
                                        child.child_by_field_name("name").unwrap_or(child);
                                    let alias =
                                        child.child_by_field_name("alias").unwrap_or(original);
                                    imports.push((
                                        path.clone(),
                                        text(source, original).to_string(),
                                        text(source, alias).to_string(),
                                    ));
                                }
                            }
                        }
                    }
                }
                ("javascript" | "typescript", "import_statement") => {
                    if let Some(path) = n
                        .child_by_field_name("source")
                        .and_then(|n| string(source, n))
                        .and_then(|n| module_path(root, inputs, file, &n))
                    {
                        let mut nodes = children(n);
                        while let Some(spec) = nodes.pop() {
                            if spec.kind() == "import_specifier" {
                                if let Some(original) = spec.child_by_field_name("name") {
                                    let alias =
                                        spec.child_by_field_name("alias").unwrap_or(original);
                                    imports.push((
                                        path.clone(),
                                        text(source, original).to_string(),
                                        text(source, alias).to_string(),
                                    ));
                                }
                            } else {
                                nodes.extend(children(spec));
                            }
                        }
                    }
                }
                ("rust", "use_declaration") => {
                    if let Some(arg) = n.child_by_field_name("argument") {
                        let original = if arg.kind() == "use_as_clause" {
                            arg.child_by_field_name("path").unwrap_or(arg)
                        } else {
                            arg
                        };
                        if let Some((module, name)) = text(source, original).rsplit_once("::") {
                            if let Some(path) = module_path(root, inputs, file, module) {
                                let alias = arg
                                    .child_by_field_name("alias")
                                    .map(|n| text(source, n))
                                    .unwrap_or(name);
                                imports.push((path, name.to_string(), alias.to_string()));
                            }
                        }
                    }
                }
                _ => {}
            }
            for (path, name, alias) in imports {
                if let Some(rel) = path.strip_prefix(root).ok().and_then(|p| p.to_str()) {
                    if binding(graph, root, &format!("{rel}::{name}")) == Some(authority) {
                        names.insert(alias);
                    }
                }
            }
        }
        let mut pending = vec![tree.root_node()];
        while let Some(node) = pending.pop() {
            if matches!(node.kind(), "call" | "call_expression") {
                if let Some(callee) = node
                    .child_by_field_name("function")
                    .filter(|n| n.kind() == "identifier")
                {
                    let name = text(source, callee);
                    if names.contains(name) && !shadowed(node, source, name) {
                        let key = path_key(root, &file.path);
                        consumers.push(json!({"range":offset_range(&key,source,node.start_byte() as u64,node.end_byte() as u64),"kind":"call","binding":base(root,input(inputs,&target.location.file).unwrap(),target),"local_name":name}));
                    }
                }
            }
            pending.extend(children(node));
        }
    }
    consumers.sort_by_key(|v| serde_json::to_string(v).unwrap());
    consumers.dedup();
    consumers
}

pub(crate) fn append(
    root: &Path,
    config: &Config,
    inputs: &[InputRecord],
    graph: &Graph,
    findings: &mut Vec<Value>,
    losses: &mut Vec<Value>,
) {
    let Some(policy) = &config.semantic_policy else {
        return;
    };
    if policy.layers.is_empty() && policy.migrations.is_empty() && policy.exports.is_empty() {
        return;
    }
    if policy.version != 1 {
        for pattern in OWNED {
            debt(
                pattern,
                root,
                None,
                "policy_version_unresolved",
                json!(policy),
                losses,
            );
        }
        return;
    }
    let mut exports = Exports {
        root,
        inputs,
        graph,
        active: BTreeSet::new(),
    };
    for rule in &policy.layers {
        let (Ok(from), Ok(to)) = (glob::Pattern::new(&rule.from), glob::Pattern::new(&rule.to))
        else {
            debt(
                "layer_violation",
                root,
                None,
                "policy_layer_malformed",
                json!(rule),
                losses,
            );
            continue;
        };
        for file in inputs.iter().filter(|i| i.disposition == "selected") {
            let relative = file
                .path
                .strip_prefix(root)
                .unwrap_or(&file.path)
                .to_string_lossy();
            if !from.matches(&relative) {
                continue;
            }
            let surface = exports.surface(&file.path);
            let mut edges: BTreeMap<PathBuf, (usize, usize)> = BTreeMap::new();
            for r in inputs
                .iter()
                .flat_map(|i| graph.references_from_file(&i.path))
                .filter(|r| {
                    r.location.file == file.path
                        && r.kind == ReferenceKind::Import
                        && r.resolution == ResolutionStatus::Resolved
                })
            {
                if let Some(target) = graph
                    .get_symbol(r.to)
                    .filter(|s| s.location.file != file.path)
                {
                    let content = file.content.as_deref().unwrap_or("");
                    let range = location_range(&path_key(root, &file.path), content, &r.location);
                    edges.entry(target.location.file.clone()).or_insert((
                        range["start_byte"].as_u64().unwrap() as usize,
                        range["end_byte"].as_u64().unwrap() as usize,
                    ));
                }
            }
            match surface {
                Ok(s) => {
                    for (target, range) in s.dependencies {
                        edges.entry(target).or_insert(range);
                    }
                }
                Err(reason) => debt(
                    "layer_violation",
                    root,
                    Some(file),
                    reason,
                    json!(rule),
                    losses,
                ),
            }
            for (target, (a, b)) in edges {
                if !to.matches(
                    &target
                        .strip_prefix(root)
                        .unwrap_or(&target)
                        .to_string_lossy(),
                ) {
                    continue;
                }
                let key = path_key(root, &file.path);
                let target_key = path_key(root, &target);
                let content = file.content.as_deref().unwrap_or("");
                let subject = json!({"path_key":key,"language":file.language,"qualified_name":relative,"construct_kind":"module","name":relative});
                findings.push(finding("layer_violation",subject,offset_range(&key,content,a as u64,b as u64),content,json!(rule),"declared-forbidden-module-dependency",json!({"kind":"resolved_module_edge","from_path_key":key,"to_path_key":target_key,"policy":rule})));
            }
        }
    }
    for rule in &policy.migrations {
        let old = binding(graph, root, &rule.old);
        let new = binding(graph, root, &rule.new);
        let (Some(old), Some(new)) = (old, new) else {
            debt(
                "incomplete_migration",
                root,
                None,
                "policy_binding_unresolved",
                json!(rule),
                losses,
            );
            continue;
        };
        if old == new {
            debt(
                "incomplete_migration",
                root,
                None,
                "policy_binding_unresolved",
                json!(rule),
                losses,
            );
            continue;
        }
        let symbol = graph.get_symbol(old).unwrap();
        let Some(file) = input(inputs, &symbol.location.file) else {
            debt(
                "incomplete_migration",
                root,
                None,
                "policy_binding_unresolved",
                json!(rule),
                losses,
            );
            continue;
        };
        let consumers = live_calls(root, inputs, graph, old);
        if consumers.is_empty() {
            continue;
        }
        let key = path_key(root, &file.path);
        let content = file.content.as_deref().unwrap();
        let mut subject = base(root, file, symbol);
        subject["name"] = json!(symbol.name);
        let new_symbol = graph.get_symbol(new).unwrap();
        let new_file = input(inputs, &new_symbol.location.file).unwrap();
        findings.push(finding("incomplete_migration",subject,location_range(&key,content,&symbol.location),content,json!(rule),"declared-old-authority-has-live-consumers",json!({"kind":"exact_migration_consumers","old_binding":base(root,file,symbol),"new_binding":base(root,new_file,new_symbol),"live_old_consumers":consumers })));
    }
    for rule in &policy.exports {
        let path = root.join(&rule.module);
        let file = input(inputs, &path);
        let surface = exports.surface(&path);
        let Ok(surface) = surface else {
            debt(
                "missing_reexport",
                root,
                file,
                surface.err().unwrap(),
                json!(rule),
                losses,
            );
            continue;
        };
        let file = file.unwrap();
        // A grammar-owned literal reexport has a complete target surface here.
        // Preserve the native unresolved record for other families; discharge
        // only this consumer's prerequisite and retain the actual proof.
        for item in losses
            .iter_mut()
            .filter(|item| item["reason"] == "unresolved_import")
        {
            let proven = item["ranges"].as_array().is_some_and(|ranges| {
                !ranges.is_empty()
                    && ranges.iter().all(|r| {
                        r["path_key"] == path_key(root, &path)
                            && surface.dependencies.iter().any(|(_, (a, b))| {
                                r["start_byte"] == *a as u64 && r["end_byte"] == *b as u64
                            })
                    })
            });
            if proven {
                if let Some(patterns) = item["affected_patterns"].as_array_mut() {
                    patterns.retain(|p| p != "missing_reexport");
                }
                item["module_policy_discharge"] = json!({"pattern":"missing_reexport","proof":"literal-source-owned-parent-export-graph","policy":rule});
                let mut identity = item.clone();
                identity.as_object_mut().unwrap().remove("id");
                item["id"] = json!(hash("loss", &identity));
            }
        }
        let key = path_key(root, &path);
        let content = file.content.as_deref().unwrap();
        for locator in &rule.symbols {
            let Some(id) = binding(graph, root, locator) else {
                debt(
                    "missing_reexport",
                    root,
                    Some(file),
                    "policy_binding_unresolved",
                    json!(rule),
                    losses,
                );
                continue;
            };
            let symbol = graph.get_symbol(id).unwrap();
            let Some(child) = input(inputs, &symbol.location.file) else {
                debt(
                    "missing_reexport",
                    root,
                    Some(file),
                    "policy_binding_unresolved",
                    json!(rule),
                    losses,
                );
                continue;
            };
            let child_surface = exports.surface(&child.path);
            let Ok(child_surface) = child_surface else {
                debt(
                    "missing_reexport",
                    root,
                    Some(child),
                    child_surface.err().unwrap(),
                    json!(rule),
                    losses,
                );
                continue;
            };
            if !child_surface.exports.values().any(|export| *export == id) {
                debt(
                    "missing_reexport",
                    root,
                    Some(child),
                    "policy_required_binding_private",
                    json!(rule),
                    losses,
                );
                continue;
            }
            if surface.exports.values().any(|export| *export == id) {
                continue;
            }
            let subject = json!({"path_key":key,"language":file.language,"qualified_name":rule.module,"construct_kind":"module","name":rule.module});
            findings.push(finding("missing_reexport",subject,offset_range(&key,content,0,content.len() as u64),content,json!({"module":rule.module,"required_binding":locator}),"declared-public-binding-absent-from-parent-export-graph",json!({"kind":"complete_parent_export_absence","required_binding":base(root,child,symbol),"required_range":location_range(&path_key(root,&child.path),child.content.as_deref().unwrap(),&symbol.location),"parent_exports":surface.exports.iter().map(|(alias,id)|json!({"alias":alias,"binding":graph.get_symbol(*id).and_then(|s|input(inputs,&s.location.file).map(|i|base(root,i,s)))})).collect::<Vec<_>>() })));
        }
    }
}
