//! Per-file facts: everything the resolver needs from one Python file,
//! extracted in one tree-sitter pass and serialised before the next file is
//! read. A fact set depends on nothing outside its file: cross-file names
//! stay as text until the resolution pass.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use tree_sitter::{Node, Parser, Tree};

/// The shape of an expression, as far as receiver typing needs it.
///
/// Argument lists, literals and operators are not kept: only the chain of
/// names, attributes and calls that decides what a value may be.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Expr {
    /// A bare identifier.
    Name(String),
    /// `object.attr`.
    Attr(Box<Expr>, String),
    /// `callee(...)`; the callee's result is the value.
    Call(Box<Expr>),
    /// `super()`.
    Super,
    /// `with <expr> as x`: the value bound to `x` is `<expr>.__enter__()`.
    Enter(Box<Expr>),
    /// One of several shapes (`a if c else b`, `a or b`).
    Either(Vec<Expr>),
    /// The literal `None`.
    NoneLit,
    /// A literal, comprehension or operator: a value, but never a project type.
    Known,
    /// Anything the shape language does not describe.
    Unknown,
}

/// The kind of a definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DefKind {
    /// The module scope of the file.
    Module,
    /// A function not directly inside a class body.
    Function,
    /// A function directly inside a class body.
    Method,
    /// A class.
    Class,
    /// A module-level name bound by assignment (a variable or, when spelled
    /// in upper case, a constant).
    Variable,
}

/// One definition in a file. Indices (`parent`, `class`) are positions in
/// [`FactSet::defs`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefFact {
    /// Simple name; `<module>` for the module scope.
    pub name: String,
    /// Name path inside the file, `Class::method`.
    pub qual: String,
    /// What kind of definition.
    pub kind: DefKind,
    /// 1-based line of the definition.
    pub line: u32,
    /// 1-based column of the definition.
    pub column: u32,
    /// 1-based line where the definition ends.
    pub end_line: u32,
    /// 1-based column where the definition ends.
    pub end_column: u32,
    /// Enclosing definition; `None` only for the module.
    pub parent: Option<u32>,
    /// Owning class of a method, or of a function nested in a method.
    pub class: Option<u32>,
    /// Decorator texts without the leading `@`.
    pub decorators: Vec<String>,
    /// Return annotation text.
    pub returns: Option<String>,
    /// Base class texts, in order.
    pub bases: Vec<String>,
    /// Parameters as (name, annotation text). The first parameter of a
    /// method is listed like any other.
    pub params: Vec<(String, Option<String>)>,
    /// When the body has exactly one `return <name>(...)`, that name.
    pub single_call_return: Option<String>,
}

/// An import binding a local name in a scope.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportFact {
    /// Scope the name is bound in (nearest enclosing non-class definition).
    pub scope: u32,
    /// The local name.
    pub local: String,
    /// Absolute dotted module (relative imports already resolved).
    pub module: String,
    /// `from module import member`; `None` for `import module`.
    pub member: Option<String>,
    /// 1-based line.
    pub line: u32,
    /// Byte span of the import statement.
    #[serde(default)]
    pub span: (u32, u32),
    /// Whether the statement was spelled relative (`from . import x`).
    #[serde(default)]
    pub relative: bool,
}

/// What a binding assigns to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum BindTarget {
    /// A local or module-level name in `scope`.
    Local(String),
    /// `self.attr = ...` inside a method of `class`.
    SelfAttr(String),
    /// `attr = ...` or `attr: T` directly in a class body.
    ClassAttr(String),
}

/// One binding of a name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindFact {
    /// Scope whose names the value is evaluated in.
    pub scope: u32,
    /// The class a `SelfAttr`/`ClassAttr` belongs to.
    pub class: Option<u32>,
    /// What is bound.
    pub target: BindTarget,
    /// The assigned value's shape.
    pub value: Expr,
    /// A declared annotation, which takes precedence over the value.
    pub annotation: Option<String>,
}

/// How a reference uses its target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SiteKind {
    /// A call expression.
    Call,
    /// The name is used as a value or read as an attribute.
    Reference,
    /// Applied as a decorator.
    Decorator,
    /// Imported.
    Import,
}

/// One place that refers to something.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefFact {
    /// Definition the site is attributed to.
    pub owner: u32,
    /// Scope names are looked up in.
    pub scope: u32,
    /// 1-based line.
    pub line: u32,
    /// 1-based column, so two sites on one line stay distinct.
    #[serde(default)]
    pub column: u32,
    /// Call, reference or decorator.
    pub kind: SiteKind,
    /// The callee (for a call) or the value expression.
    pub expr: Expr,
}

/// A construct the resolver cannot close over, recorded with its byte span
/// so the report can name the exact source it could not prove.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HazardKind {
    /// A parameter or return annotation; the contract it states is parsed
    /// but no type-flow evidence is built for it (`capability_unavailable`).
    Annotation,
    /// A bare-name call; a loss when the name resolves to a runtime
    /// namespace operation (`getattr`, `globals`, `eval`, ...) provided by
    /// the builtins, so any name may be read (`unresolved_reference`).
    DynamicNamespace,
    /// A `match` statement; pattern bindings and guards are not modelled
    /// (`unsupported_construct`).
    MatchStatement,
}

/// One hazard with its span.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hazard {
    /// What it is.
    pub kind: HazardKind,
    /// Scope the hazard sits in (for provider lookup).
    pub scope: u32,
    /// Byte span in the file.
    pub start: u32,
    /// End byte, exclusive.
    pub end: u32,
    /// The annotation text, or the called name, for later resolution.
    pub text: String,
}

/// Everything extracted from one file.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FactSet {
    /// Root-relative path with `/` separators.
    pub rel: String,
    /// Dotted module name.
    pub module: String,
    /// Whether the file is an `__init__.py`.
    pub is_package: bool,
    /// Definitions; index 0 is the module scope.
    pub defs: Vec<DefFact>,
    /// Imports.
    pub imports: Vec<ImportFact>,
    /// Modules star-imported at module level.
    pub stars: Vec<String>,
    /// Bindings.
    pub binds: Vec<BindFact>,
    /// References.
    pub refs: Vec<RefFact>,
    /// Number of call expressions seen.
    pub call_count: u32,
    /// Whether tree-sitter reported syntax errors.
    pub has_errors: bool,
    /// Constructs the resolver cannot close over.
    #[serde(default)]
    pub hazards: Vec<Hazard>,
    /// Names listed in a literal `__all__`.
    #[serde(default)]
    pub all_names: Vec<String>,
    /// String literals that spell an identifier (`"_check_rule"`): a
    /// name-based dispatch table may look the definition up by it.
    #[serde(default)]
    pub named_strings: Vec<String>,
}

/// Why a file produced no facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// The parser could not be initialised or returned no tree.
    Parse,
    /// The tree is nested deeper than the walker's limit.
    TooDeep,
}

fn text<'a>(node: Node, src: &'a str) -> &'a str {
    node.utf8_text(src.as_bytes()).unwrap_or("")
}

fn line_of(node: Node) -> u32 {
    node.start_position().row as u32 + 1
}

/// Extract the facts of one file.
///
/// `rel` is the root-relative path, `module` the dotted module name and
/// `package` the dotted package the file belongs to (used to resolve
/// relative imports).
pub fn extract(
    parser: &mut Parser,
    src: &str,
    rel: &str,
    module: &str,
    is_package: bool,
) -> Result<FactSet, SkipReason> {
    let tree = parser.parse(src, None).ok_or(SkipReason::Parse)?;
    if tree_depth_exceeds(&tree, crate::parser::MAX_AST_DEPTH) {
        return Err(SkipReason::TooDeep);
    }
    let mut facts = FactSet {
        rel: rel.to_string(),
        module: module.to_string(),
        is_package,
        has_errors: tree.root_node().has_error(),
        ..FactSet::default()
    };
    facts.defs.push(DefFact {
        name: "<module>".to_string(),
        qual: "<module>".to_string(),
        kind: DefKind::Module,
        line: 1,
        column: 1,
        end_line: 1,
        end_column: 1,
        parent: None,
        class: None,
        decorators: Vec::new(),
        returns: None,
        bases: Vec::new(),
        params: Vec::new(),
        single_call_return: None,
    });
    let mut package: Vec<String> = if module.is_empty() {
        Vec::new()
    } else {
        module.split('.').map(str::to_string).collect()
    };
    if !is_package {
        package.pop();
    }
    let mut cx = Extractor {
        src,
        facts: &mut facts,
        package,
        globals: HashMap::new(),
        shadow: Vec::new(),
    };
    cx.walk(tree.root_node(), 0, 0, 0, &mut Vec::new());
    Ok(facts)
}

/// Whether any node of `tree` sits deeper than `limit`, without recursion.
pub fn tree_depth_exceeds(tree: &Tree, limit: usize) -> bool {
    let mut cursor = tree.walk();
    let mut depth = 0usize;
    loop {
        if cursor.goto_first_child() {
            depth += 1;
            if depth > limit {
                return true;
            }
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return false;
            }
            depth -= 1;
        }
    }
}

struct Extractor<'a> {
    src: &'a str,
    facts: &'a mut FactSet,
    package: Vec<String>,
    /// Names declared `global` per scope: an assignment to one inside that
    /// scope rebinds the module variable instead of shadowing it.
    globals: HashMap<u32, Vec<String>>,
    /// Names bound by the lambdas enclosing the node being walked; a read
    /// of one is the parameter, not an outer definition of the same name.
    shadow: Vec<String>,
}

impl Extractor<'_> {
    /// The shape of an expression node.
    fn shape(&self, node: Node) -> Expr {
        match node.kind() {
            "identifier" => Expr::Name(text(node, self.src).to_string()),
            "attribute" => match (
                node.child_by_field_name("object"),
                node.child_by_field_name("attribute"),
            ) {
                (Some(o), Some(a)) => {
                    Expr::Attr(Box::new(self.shape(o)), text(a, self.src).to_string())
                }
                _ => Expr::Unknown,
            },
            "call" => match node.child_by_field_name("function") {
                Some(f) if f.kind() == "identifier" && text(f, self.src) == "super" => Expr::Super,
                Some(f) => Expr::Call(Box::new(self.shape(f))),
                None => Expr::Unknown,
            },
            "await" | "parenthesized_expression" => match node.named_child(0) {
                Some(inner) => self.shape(inner),
                None => Expr::Unknown,
            },
            "conditional_expression" => {
                let mut v = Vec::new();
                for i in [0, 2] {
                    v.push(match node.named_child(i) {
                        Some(n) => self.shape(n),
                        None => Expr::Unknown,
                    });
                }
                Expr::Either(v)
            }
            "boolean_operator" => {
                let mut v = Vec::new();
                for field in ["left", "right"] {
                    v.push(match node.child_by_field_name(field) {
                        Some(n) => self.shape(n),
                        None => Expr::Unknown,
                    });
                }
                Expr::Either(v)
            }
            "named_expression" => match node.child_by_field_name("value") {
                Some(n) => self.shape(n),
                None => Expr::Unknown,
            },
            "none" => Expr::NoneLit,
            "string"
            | "concatenated_string"
            | "integer"
            | "float"
            | "true"
            | "false"
            | "list"
            | "dictionary"
            | "set"
            | "tuple"
            | "list_comprehension"
            | "dictionary_comprehension"
            | "set_comprehension"
            | "generator_expression"
            | "comparison_operator"
            | "not_operator"
            | "binary_operator"
            | "unary_operator" => Expr::Known,
            _ => Expr::Unknown,
        }
    }

    fn add_def(&mut self, node: Node, owner: u32, decorators: Vec<String>) -> u32 {
        let src = self.src;
        let is_class = node.kind() == "class_definition";
        let name = node
            .child_by_field_name("name")
            .map(|n| text(n, src).to_string())
            .unwrap_or_default();
        let parent = &self.facts.defs[owner as usize];
        let qual = if parent.kind == DefKind::Module {
            name.clone()
        } else {
            format!("{}::{}", parent.qual, name)
        };
        let kind = if is_class {
            DefKind::Class
        } else if parent.kind == DefKind::Class {
            DefKind::Method
        } else {
            DefKind::Function
        };
        let class = if is_class {
            None
        } else if parent.kind == DefKind::Class {
            Some(owner)
        } else {
            parent.class
        };
        let returns = node
            .child_by_field_name("return_type")
            .map(|n| text(n, src).to_string());
        if let Some(n) = node.child_by_field_name("return_type") {
            self.hazard(HazardKind::Annotation, owner, n);
        }
        let mut bases = Vec::new();
        if let Some(sup) = node.child_by_field_name("superclasses") {
            let mut c = sup.walk();
            for b in sup.named_children(&mut c) {
                if b.kind() != "keyword_argument" {
                    bases.push(text(b, src).to_string());
                }
            }
        }
        let mut params = Vec::new();
        if let Some(ps) = node.child_by_field_name("parameters") {
            let mut c = ps.walk();
            for p in ps.named_children(&mut c) {
                match p.kind() {
                    "identifier" => params.push((text(p, src).to_string(), None)),
                    "typed_parameter" | "typed_default_parameter" => {
                        let n = p
                            .child_by_field_name("name")
                            .or_else(|| p.named_child(0))
                            .filter(|n| n.kind() == "identifier");
                        if let Some(n) = n {
                            params.push((
                                text(n, src).to_string(),
                                p.child_by_field_name("type")
                                    .map(|t| text(t, src).to_string()),
                            ));
                            if let Some(t) = p.child_by_field_name("type") {
                                self.hazard(HazardKind::Annotation, owner, t);
                            }
                        }
                    }
                    "default_parameter" => {
                        if let Some(n) = p.child_by_field_name("name") {
                            params.push((text(n, src).to_string(), None));
                        }
                    }
                    "list_splat_pattern" | "dictionary_splat_pattern" => {
                        if let Some(n) = p.named_child(0).filter(|n| n.kind() == "identifier") {
                            params.push((text(n, src).to_string(), None));
                        }
                    }
                    _ => {}
                }
            }
        }
        let single_call_return = if is_class {
            None
        } else {
            node.child_by_field_name("body").and_then(|body| {
                let mut returns = Vec::new();
                collect_returns(body, &mut returns);
                if returns.len() != 1 {
                    return None;
                }
                let value = returns[0].named_child(0)?;
                if value.kind() != "call" {
                    return None;
                }
                let func = value.child_by_field_name("function")?;
                (func.kind() == "identifier").then(|| text(func, src).to_string())
            })
        };
        let id = self.facts.defs.len() as u32;
        self.facts.defs.push(DefFact {
            name,
            qual,
            kind,
            line: line_of(node),
            column: node.start_position().column as u32 + 1,
            end_line: node.end_position().row as u32 + 1,
            end_column: node.end_position().column as u32 + 1,
            parent: Some(owner),
            class,
            decorators,
            returns,
            bases,
            params,
            single_call_return,
        });
        id
    }

    fn bind_pattern(&mut self, node: Node, scope: u32, value: Expr) {
        match node.kind() {
            "identifier" => {
                let name = text(node, self.src).to_string();
                let scope = self.binding_scope(scope, &name);
                self.facts.binds.push(BindFact {
                    scope,
                    class: None,
                    target: BindTarget::Local(name),
                    value,
                    annotation: None,
                })
            }
            "pattern_list"
            | "tuple_pattern"
            | "list_pattern"
            | "list_splat_pattern"
            | "as_pattern_target"
            | "parenthesized_expression"
            | "tuple"
            | "list" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.bind_pattern(ch, scope, Expr::Unknown);
                }
            }
            _ => {}
        }
    }

    fn add_import(&mut self, node: Node, scope: u32) {
        let src = self.src;
        let line = line_of(node);
        let span = (node.start_byte() as u32, node.end_byte() as u32);
        if node.kind() == "import_statement" {
            let mut c = node.walk();
            for n in node.children_by_field_name("name", &mut c) {
                let (local, module) = if n.kind() == "aliased_import" {
                    (
                        n.child_by_field_name("alias")
                            .map(|x| text(x, src).to_string())
                            .unwrap_or_default(),
                        n.child_by_field_name("name")
                            .map(|x| text(x, src).to_string())
                            .unwrap_or_default(),
                    )
                } else {
                    let first = text(n, src).split('.').next().unwrap_or("").to_string();
                    (first.clone(), first)
                };
                if !local.is_empty() {
                    self.facts.imports.push(ImportFact {
                        scope,
                        local,
                        module,
                        member: None,
                        line,
                        span,
                        relative: false,
                    });
                }
            }
            return;
        }
        let Some(mn) = node.child_by_field_name("module_name") else {
            return;
        };
        let raw = text(mn, src).trim();
        let level = raw.chars().take_while(|c| *c == '.').count();
        let rest = &raw[level..];
        let module = if level == 0 {
            rest.to_string()
        } else {
            let mut pkg = self.package.clone();
            for _ in 1..level {
                pkg.pop();
            }
            if !rest.is_empty() {
                pkg.extend(rest.split('.').map(str::to_string));
            }
            pkg.join(".")
        };
        let mut c = node.walk();
        let mut any = false;
        for n in node.children_by_field_name("name", &mut c) {
            any = true;
            let (member, local) = if n.kind() == "aliased_import" {
                (
                    n.child_by_field_name("name")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default(),
                    n.child_by_field_name("alias")
                        .map(|x| text(x, src).to_string())
                        .unwrap_or_default(),
                )
            } else {
                let t = text(n, src).to_string();
                (t.clone(), t)
            };
            if !local.is_empty() {
                self.facts.imports.push(ImportFact {
                    scope,
                    local,
                    module: module.clone(),
                    member: Some(member),
                    line,
                    span,
                    relative: level > 0,
                });
            }
        }
        if !any {
            self.facts.stars.push(module);
        }
    }

    /// Whether the root name of `expr` is a parameter of an enclosing lambda.
    fn shadowed(&self, expr: &Expr) -> bool {
        let mut e = expr;
        loop {
            match e {
                Expr::Name(n) => return self.shadow.iter().any(|s| s == n),
                Expr::Attr(inner, _) | Expr::Call(inner) => e = inner,
                _ => return false,
            }
        }
    }

    /// The scope an assignment to `name` in `scope` binds: the module scope
    /// when the name was declared `global` there.
    fn binding_scope(&self, scope: u32, name: &str) -> u32 {
        if self
            .globals
            .get(&scope)
            .is_some_and(|g| g.iter().any(|n| n == name))
        {
            0
        } else {
            scope
        }
    }

    /// `__all__ = ["a", "b"]` (or `+= [...]`): each string is a module-level
    /// read of that name; the package exports it.
    fn walk_all_entries(&mut self, node: Node, owner: u32) {
        if !matches!(node.kind(), "list" | "tuple") {
            return;
        }
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            if matches!(ch.kind(), "string") {
                let name = text(ch, self.src)
                    .trim_matches(|c| c == '"' || c == '\'')
                    .to_string();
                if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                    self.facts.all_names.push(name.clone());
                    let shape = Expr::Name(name);
                    self.push_ref(owner, 0, ch, SiteKind::Reference, shape);
                }
            }
        }
    }

    /// Record every name an annotation reads. A string inside an annotation
    /// is a forward reference (`"Foo | None"`): its identifiers are reads too.
    fn walk_annotation(&mut self, node: Node, scope: u32, owner: u32) {
        match node.kind() {
            "string" | "concatenated_string" => {
                let raw = text(node, self.src);
                let inner = raw.trim_matches(|c| c == '"' || c == '\'');
                let mut cur = String::new();
                let flush = |cur: &mut String, this: &mut Self| {
                    if !cur.is_empty() && !cur.starts_with(|c: char| c.is_ascii_digit()) {
                        let shape = Expr::Name(std::mem::take(cur));
                        this.push_ref(owner, scope, node, SiteKind::Reference, shape);
                    }
                    cur.clear();
                };
                // Only the head of a dotted name is a binding in this file:
                // after a `.` skip to the next separator.
                let mut skipping = false;
                for c in inner.chars() {
                    if c.is_alphanumeric() || c == '_' {
                        if !skipping {
                            cur.push(c);
                        }
                    } else {
                        flush(&mut cur, self);
                        skipping = c == '.';
                    }
                }
                flush(&mut cur, self);
            }
            "identifier" => {
                let shape = Expr::Name(text(node, self.src).to_string());
                self.push_ref(owner, scope, node, SiteKind::Reference, shape);
            }
            "attribute" => {
                // `typing.Optional[Foo]`: the head is the read; walk the rest
                // for subscripts that may still name local types.
                let shape = self.shape(node);
                self.push_ref(owner, scope, node, SiteKind::Reference, shape);
            }
            _ => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.walk_annotation(ch, scope, owner);
                }
            }
        }
    }

    fn hazard(&mut self, kind: HazardKind, scope: u32, node: Node) {
        self.facts.hazards.push(Hazard {
            kind,
            scope,
            start: node.start_byte() as u32,
            end: node.end_byte() as u32,
            text: text(node, self.src).to_string(),
        });
    }

    fn push_ref(&mut self, owner: u32, scope: u32, node: Node, kind: SiteKind, expr: Expr) {
        if self.shadowed(&expr) {
            return;
        }
        self.facts.refs.push(RefFact {
            owner,
            scope,
            line: line_of(node),
            column: node.start_position().column as u32 + 1,
            kind,
            expr,
        });
    }

    /// Walk `node`. `scope` is the nearest enclosing non-class definition
    /// (where names are looked up), `owner` the definition sites are
    /// attributed to, `class` the enclosing class body if any.
    fn walk(&mut self, node: Node, scope: u32, owner: u32, class: u32, pending: &mut Vec<String>) {
        let src = self.src;
        match node.kind() {
            "import_statement" | "import_from_statement" => {
                self.add_import(node, scope);
                return;
            }
            "parameters" | "lambda_parameters" | "comment" => return,
            // An annotation reads the names in it (`x: Foo`, `-> Bar`,
            // `Optional[Baz]`); a quoted forward reference is one too.
            "type" => {
                self.walk_annotation(node, scope, owner);
                return;
            }
            "lambda" => {
                // Defaults run in the enclosing scope; the body sees the
                // parameters first.
                let mut names = Vec::new();
                if let Some(ps) = node.child_by_field_name("parameters") {
                    let mut c = ps.walk();
                    for p in ps.named_children(&mut c) {
                        let name = match p.kind() {
                            "identifier" => Some(p),
                            "default_parameter" | "typed_default_parameter" => {
                                if let Some(v) = p.child_by_field_name("value") {
                                    self.walk(v, scope, owner, class, &mut Vec::new());
                                }
                                p.child_by_field_name("name")
                            }
                            "typed_parameter"
                            | "list_splat_pattern"
                            | "dictionary_splat_pattern" => {
                                p.named_child(0).filter(|n| n.kind() == "identifier")
                            }
                            _ => None,
                        };
                        if let Some(n) = name {
                            names.push(text(n, src).to_string());
                        }
                    }
                }
                let depth = self.shadow.len();
                self.shadow.extend(names);
                if let Some(body) = node.child_by_field_name("body") {
                    self.walk(body, scope, owner, class, &mut Vec::new());
                }
                self.shadow.truncate(depth);
                return;
            }
            "decorated_definition" => {
                let mut decs = Vec::new();
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "decorator" {
                        decs.push(text(ch, src).trim_start_matches('@').trim().to_string());
                    }
                }
                let Some(def) = node.child_by_field_name("definition") else {
                    return;
                };
                let id = self.walk_def(def, scope, owner, class, decs);
                // Decorator expressions run in the enclosing scope and are
                // attributed to the decorated definition.
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() != "decorator" {
                        continue;
                    }
                    let Some(expr) = ch.named_child(0) else {
                        continue;
                    };
                    match expr.kind() {
                        "identifier" | "attribute" => {
                            let shape = self.shape(expr);
                            self.push_ref(id, scope, expr, SiteKind::Decorator, shape);
                            if let Some(obj) = expr.child_by_field_name("object") {
                                self.walk(obj, scope, id, class, &mut Vec::new());
                            }
                        }
                        _ => self.walk(expr, scope, id, class, &mut Vec::new()),
                    }
                }
                return;
            }
            "function_definition" | "class_definition" => {
                let decs = std::mem::take(pending);
                self.walk_def(node, scope, owner, class, decs);
                return;
            }
            "match_statement" => {
                self.hazard(HazardKind::MatchStatement, scope, node);
            }
            "global_statement" => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    if ch.kind() == "identifier" {
                        self.globals
                            .entry(scope)
                            .or_default()
                            .push(text(ch, src).to_string());
                    }
                }
                return;
            }
            "call" => {
                self.facts.call_count += 1;
                if let Some(func) = node.child_by_field_name("function") {
                    match func.kind() {
                        "identifier" | "attribute" => {
                            let shape = self.shape(func);
                            if let Expr::Name(n) = &shape {
                                // Directly, or through an import alias
                                // (`from builtins import getattr as lookup`):
                                // the resolver, which has the import table,
                                // decides whether this is a namespace operation.
                                if !self.shadowed(&shape)
                                    && n.chars().all(|c| c.is_alphanumeric() || c == '_')
                                {
                                    self.hazard(HazardKind::DynamicNamespace, scope, node);
                                }
                            }
                            self.push_ref(owner, scope, node, SiteKind::Call, shape);
                            if let Some(obj) = func.child_by_field_name("object") {
                                self.walk(obj, scope, owner, class, &mut Vec::new());
                            }
                        }
                        _ => self.walk(func, scope, owner, class, &mut Vec::new()),
                    }
                }
                if let Some(args) = node.child_by_field_name("arguments") {
                    self.walk(args, scope, owner, class, &mut Vec::new());
                }
                return;
            }
            "keyword_argument" => {
                if let Some(v) = node.child_by_field_name("value") {
                    self.walk(v, scope, owner, class, &mut Vec::new());
                }
                return;
            }
            "assignment" | "augmented_assignment" => {
                let left = node.child_by_field_name("left");
                let right = node.child_by_field_name("right");
                let annotation = node
                    .child_by_field_name("type")
                    .map(|t| text(t, src).to_string());
                if let Some(t) = node.child_by_field_name("type") {
                    self.walk_annotation(t, scope, owner);
                }
                let value = match (node.kind(), right) {
                    ("assignment", Some(r)) => self.shape(r),
                    _ => Expr::Unknown,
                };
                if let Some(left) = left {
                    let in_class_body = owner == class && class != 0 && scope != class;
                    if left.kind() == "identifier" && text(left, src) == "__all__" && scope == 0 {
                        if let Some(r) = right {
                            self.walk_all_entries(r, owner);
                        }
                    }
                    match left.kind() {
                        "identifier" => {
                            let name = text(left, src).to_string();
                            // A first module-level assignment defines a variable entity.
                            if scope == 0 && owner == 0 && node.kind() == "assignment" {
                                let exists = self.facts.defs.iter().any(|d| {
                                    d.kind == DefKind::Variable
                                        && d.parent == Some(0)
                                        && d.name == name
                                });
                                if !exists && name != "__all__" {
                                    self.facts.defs.push(DefFact {
                                        name: name.clone(),
                                        qual: name.clone(),
                                        kind: DefKind::Variable,
                                        line: line_of(node),
                                        column: node.start_position().column as u32 + 1,
                                        end_line: node.end_position().row as u32 + 1,
                                        end_column: node.end_position().column as u32 + 1,
                                        parent: Some(0),
                                        class: None,
                                        decorators: Vec::new(),
                                        returns: None,
                                        bases: Vec::new(),
                                        params: Vec::new(),
                                        single_call_return: None,
                                    });
                                }
                            }
                            if in_class_body {
                                self.facts.binds.push(BindFact {
                                    scope,
                                    class: Some(class),
                                    target: BindTarget::ClassAttr(name),
                                    value,
                                    annotation,
                                });
                            } else {
                                let scope = self.binding_scope(scope, &name);
                                self.facts.binds.push(BindFact {
                                    scope,
                                    class: None,
                                    target: BindTarget::Local(name),
                                    value,
                                    annotation,
                                });
                            }
                        }
                        "attribute" => {
                            let obj = left.child_by_field_name("object");
                            let attr = left.child_by_field_name("attribute");
                            if let (Some(o), Some(a)) = (obj, attr) {
                                let owner_class = self.facts.defs[scope as usize].class;
                                if o.kind() == "identifier" && owner_class.is_some() {
                                    let first_param = self.facts.defs[scope as usize]
                                        .params
                                        .first()
                                        .map(|p| p.0.clone());
                                    if first_param.as_deref() == Some(text(o, src)) {
                                        self.facts.binds.push(BindFact {
                                            scope,
                                            class: owner_class,
                                            target: BindTarget::SelfAttr(text(a, src).to_string()),
                                            value,
                                            annotation,
                                        });
                                    }
                                }
                                self.walk(o, scope, owner, class, &mut Vec::new());
                            }
                        }
                        _ => self.walk_target(left, scope, owner, class),
                    }
                }
                if let Some(r) = right {
                    self.walk(r, scope, owner, class, &mut Vec::new());
                }
                return;
            }
            "with_item" => {
                if let Some(value) = node.child_by_field_name("value") {
                    if value.kind() == "as_pattern" {
                        let target = value
                            .child_by_field_name("alias")
                            .and_then(|a| a.named_child(0));
                        if let (Some(expr), Some(t)) = (value.named_child(0), target) {
                            let shape = Expr::Enter(Box::new(self.shape(expr)));
                            self.bind_pattern(t, scope, shape);
                            self.walk(expr, scope, owner, class, &mut Vec::new());
                            return;
                        }
                    }
                }
            }
            "for_statement" | "for_in_clause" => {
                if let Some(left) = node.child_by_field_name("left") {
                    self.bind_pattern(left, scope, Expr::Unknown);
                }
            }
            "as_pattern" => {
                if let Some(alias) = node.child_by_field_name("alias") {
                    if let Some(n) = alias.named_child(0) {
                        self.bind_pattern(n, scope, Expr::Unknown);
                    }
                }
            }
            "named_expression" => {
                if let (Some(n), Some(v)) = (
                    node.child_by_field_name("name"),
                    node.child_by_field_name("value"),
                ) {
                    let shape = self.shape(v);
                    self.bind_pattern(n, scope, shape);
                }
            }
            "attribute" => {
                let shape = self.shape(node);
                self.push_ref(owner, scope, node, SiteKind::Reference, shape);
                if let Some(obj) = node.child_by_field_name("object") {
                    self.walk(obj, scope, owner, class, &mut Vec::new());
                }
                return;
            }
            "identifier" => {
                let shape = Expr::Name(text(node, src).to_string());
                self.push_ref(owner, scope, node, SiteKind::Reference, shape);
                return;
            }
            "string" => {
                // A plain string that spells an identifier may be a
                // name-based dispatch key. An f-string holds expressions;
                // fall through so its interpolations are walked.
                let raw = text(node, src);
                let inner = raw.trim_matches(|c| c == '"' || c == '\'');
                let identifier_shaped = !raw.starts_with(['f', 'F'])
                    && !inner.is_empty()
                    && inner.len() <= 64
                    && inner.chars().all(|c| c.is_alphanumeric() || c == '_')
                    && !inner.starts_with(|c: char| c.is_ascii_digit());
                if identifier_shaped {
                    self.facts.named_strings.push(inner.to_string());
                }
                // A string inside a subscript (`Callable[[str], "Backend"]`,
                // `Optional["Foo"]`) is a forward reference: its names are
                // reads, exactly as in a `type` node.
                let in_subscript = node
                    .parent()
                    .is_some_and(|p| matches!(p.kind(), "subscript" | "list" | "tuple"))
                    && {
                        let mut a = node.parent();
                        let mut found = false;
                        while let Some(x) = a {
                            if x.kind() == "subscript" {
                                found = true;
                                break;
                            }
                            if !matches!(x.kind(), "list" | "tuple") {
                                break;
                            }
                            a = x.parent();
                        }
                        found
                    };
                if in_subscript && !raw.starts_with(['f', 'F']) {
                    self.walk_annotation(node, scope, owner);
                    return;
                }
                if identifier_shaped {
                    return;
                }
            }
            _ => {}
        }
        let mut c = node.walk();
        for ch in node.named_children(&mut c) {
            self.walk(ch, scope, owner, class, &mut Vec::new());
        }
    }

    /// Record a definition and walk its body; returns the definition index.
    fn walk_def(
        &mut self,
        node: Node,
        scope: u32,
        owner: u32,
        class: u32,
        decs: Vec<String>,
    ) -> u32 {
        let id = self.add_def(node, owner, decs);
        let is_class = node.kind() == "class_definition";
        if is_class {
            if let Some(sup) = node.child_by_field_name("superclasses") {
                let mut c = sup.walk();
                for b in sup.named_children(&mut c) {
                    if b.kind() == "keyword_argument" {
                        continue;
                    }
                    let shape = self.shape(b);
                    self.push_ref(id, scope, b, SiteKind::Reference, shape);
                }
            }
            if let Some(body) = node.child_by_field_name("body") {
                // Class bodies keep the enclosing lookup scope; sites inside
                // them are attributed to the class.
                self.walk(body, scope, id, id, &mut Vec::new());
            }
        } else {
            // Default values and annotations run in the enclosing scope.
            if let Some(ps) = node.child_by_field_name("parameters") {
                let mut c = ps.walk();
                for p in ps.named_children(&mut c) {
                    if let Some(v) = p.child_by_field_name("value") {
                        self.walk(v, scope, owner, class, &mut Vec::new());
                    }
                    if let Some(t) = p.child_by_field_name("type") {
                        self.walk_annotation(t, scope, owner);
                    }
                }
            }
            if let Some(t) = node.child_by_field_name("return_type") {
                self.walk_annotation(t, scope, owner);
            }
            // Every parameter shadows outer names; annotated ones are typed
            // when the scope's environment is seeded.
            let params: Vec<(String, Option<String>)> = self.facts.defs[id as usize].params.clone();
            for (name, _) in params {
                self.facts.binds.push(BindFact {
                    scope: id,
                    class: None,
                    target: BindTarget::Local(name),
                    value: Expr::Unknown,
                    annotation: None,
                });
            }
            if let Some(body) = node.child_by_field_name("body") {
                self.walk(body, id, id, class, &mut Vec::new());
            }
        }
        id
    }

    /// Assignment targets: only their sub-expressions are uses.
    fn walk_target(&mut self, node: Node, scope: u32, owner: u32, class: u32) {
        match node.kind() {
            "identifier" => self.bind_pattern(node, scope, Expr::Unknown),
            "attribute" => {
                if let Some(obj) = node.child_by_field_name("object") {
                    self.walk(obj, scope, owner, class, &mut Vec::new());
                }
            }
            "subscript" => self.walk(node, scope, owner, class, &mut Vec::new()),
            _ => {
                let mut c = node.walk();
                for ch in node.named_children(&mut c) {
                    self.walk_target(ch, scope, owner, class);
                }
            }
        }
    }
}

fn collect_returns<'t>(node: Node<'t>, out: &mut Vec<Node<'t>>) {
    match node.kind() {
        "function_definition" | "class_definition" | "lambda" => return,
        "return_statement" => out.push(node),
        _ => {}
    }
    let mut c = node.walk();
    for ch in node.named_children(&mut c) {
        collect_returns(ch, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(src: &str) -> FactSet {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        extract(&mut parser, src, "pkg/m.py", "pkg.m", false).unwrap()
    }

    #[test]
    fn annotations_all_entries_and_dispatch_strings_are_recorded() {
        let f = facts(
            "from typing import Optional\nimport typing\n__all__ = [\"Known\", \"api\"]\nTABLE = {\"rule\": \"_check_rule\"}\n\nclass Known:\n    pass\n\ndef api(a: Known, b: Optional[\"Known | None\"], c: typing.Any = None) -> \"pkg.other.Thing\":\n    return a\n",
        );
        let reads: Vec<(u32, String)> = f
            .refs
            .iter()
            .filter(|r| r.kind == SiteKind::Reference)
            .filter_map(|r| match &r.expr {
                Expr::Name(n) => Some((r.line, n.clone())),
                Expr::Attr(o, a) => match &**o {
                    Expr::Name(n) => Some((r.line, format!("{n}.{a}"))),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        // Parameter annotations, a quoted forward reference inside a
        // subscript, a dotted annotation head, and the return annotation.
        assert!(reads.contains(&(9, "Known".into())), "{reads:?}");
        assert!(reads.contains(&(9, "Optional".into())), "{reads:?}");
        assert_eq!(
            reads
                .iter()
                .filter(|(l, n)| *l == 9 && n == "Known")
                .count(),
            2,
            "{reads:?}"
        );
        assert!(reads.contains(&(9, "typing.Any".into())), "{reads:?}");
        assert!(reads.contains(&(9, "pkg".into())), "{reads:?}");
        assert!(
            !reads.iter().any(|(_, n)| n == "other" || n == "Thing"),
            "{reads:?}"
        );
        // `__all__` entries are module-level reads and are listed.
        assert!(
            reads.contains(&(3, "Known".into())) && reads.contains(&(3, "api".into())),
            "{reads:?}"
        );
        assert_eq!(f.all_names, vec!["Known", "api"]);
        // The dispatch-table string is recorded; the key is too (it is
        // identifier-shaped), the annotation strings are not.
        assert!(
            f.named_strings.contains(&"_check_rule".to_string()),
            "{:?}",
            f.named_strings
        );
        assert!(f.named_strings.contains(&"rule".to_string()));
        assert!(!f
            .named_strings
            .iter()
            .any(|s| s.contains(' ') || s.contains('.')));
    }

    #[test]
    fn definitions_carry_kind_parent_class_and_params() {
        let f = facts("class A:\n    def m(self, x: int):\n        def inner():\n            pass\n\ndef g(a, b=1, *args, **kw):\n    return A()\n");
        let names: Vec<(&str, DefKind, Option<u32>)> = f
            .defs
            .iter()
            .map(|d| (d.qual.as_str(), d.kind, d.class))
            .collect();
        assert_eq!(
            names,
            vec![
                ("<module>", DefKind::Module, None),
                ("A", DefKind::Class, None),
                ("A::m", DefKind::Method, Some(1)),
                ("A::m::inner", DefKind::Function, Some(1)),
                ("g", DefKind::Function, None),
            ]
        );
        assert_eq!(
            f.defs[2].params,
            vec![("self".into(), None), ("x".into(), Some("int".into()))]
        );
        assert_eq!(f.defs[4].params.len(), 4);
        assert_eq!(f.defs[4].single_call_return.as_deref(), Some("A"));
    }

    #[test]
    fn relative_imports_resolve_against_the_package() {
        let f = facts("from . import a\nfrom ..x import y as z\nimport os.path\n\ndef f():\n    from .b import c\n");
        let got: Vec<(u32, &str, &str, Option<&str>)> = f
            .imports
            .iter()
            .map(|i| {
                (
                    i.scope,
                    i.local.as_str(),
                    i.module.as_str(),
                    i.member.as_deref(),
                )
            })
            .collect();
        assert_eq!(
            got,
            vec![
                (0, "a", "pkg", Some("a")),
                (0, "z", "x", Some("y")),
                (0, "os", "os", None),
                (1, "c", "pkg.b", Some("c")),
            ]
        );
    }

    #[test]
    fn bindings_capture_shapes_and_targets() {
        let f = facts("class A:\n    limit: int = 3\n    def __init__(self, s: Sheet):\n        self.sheet = make()\n        t = s if s else None\n        with open(p) as fh:\n            pass\n        for i in rows:\n            pass\n");
        let got: Vec<(u32, &BindTarget, &Expr)> = f
            .binds
            .iter()
            .map(|b| (b.scope, &b.target, &b.value))
            .collect();
        let attr = Expr::Call(Box::new(Expr::Name("make".into())));
        assert!(got.contains(&(2, &BindTarget::SelfAttr("sheet".into()), &attr)));
        assert!(got.contains(&(0, &BindTarget::ClassAttr("limit".into()), &Expr::Known)));
        assert!(got.iter().any(|(s, t, v)| *s == 2
            && **t == BindTarget::Local("t".into())
            && matches!(v, Expr::Either(_))));
        assert!(got.iter().any(|(s, t, v)| *s == 2
            && **t == BindTarget::Local("fh".into())
            && matches!(v, Expr::Enter(_))));
        assert!(got.contains(&(2, &BindTarget::Local("i".into()), &Expr::Unknown)));
        assert!(got.contains(&(2, &BindTarget::Local("self".into()), &Expr::Unknown)));
    }

    #[test]
    fn references_record_calls_attributes_names_decorators_and_bases() {
        let f = facts("@deco\nclass B(Base):\n    pass\n\ndef f(x):\n    y = x.go(f\"{h(1)}\")\n    return y.z\n");
        let kinds: Vec<(u32, u32, SiteKind, &Expr)> = f
            .refs
            .iter()
            .map(|r| (r.owner, r.scope, r.kind, &r.expr))
            .collect();
        assert!(kinds.contains(&(1, 0, SiteKind::Decorator, &Expr::Name("deco".into()))));
        assert!(kinds.contains(&(1, 0, SiteKind::Reference, &Expr::Name("Base".into()))));
        let go = Expr::Attr(Box::new(Expr::Name("x".into())), "go".into());
        assert!(kinds.contains(&(2, 2, SiteKind::Call, &go)));
        assert!(kinds.contains(&(2, 2, SiteKind::Call, &Expr::Name("h".into()))));
        let z = Expr::Attr(Box::new(Expr::Name("y".into())), "z".into());
        assert!(kinds.contains(&(2, 2, SiteKind::Reference, &z)));
        assert_eq!(f.call_count, 2);
    }

    #[test]
    fn a_file_nested_too_deeply_is_skipped() {
        let deep = format!("x = {}1{}\n", "(".repeat(400), ")".repeat(400));
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        assert_eq!(
            extract(&mut parser, &deep, "d.py", "d", false).err(),
            Some(SkipReason::TooDeep)
        );
    }
}
