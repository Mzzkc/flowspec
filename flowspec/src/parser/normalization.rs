//! Exact, conservative whole-function normalization for the admitted public rule.
//! An unsupported mapping yields a ranged obligation, never token equivalence.
use super::semantic::SemanticIssue;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tree_sitter::Node;

pub const RULE_VERSION: &str = "duplication-normalization-v1";
pub const RULE_SHA256: &str = "8da06b2a92262e831d20a728d3f9fc8c62853b39e727009e16871a69f07f05a8";
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalizedFunction {
    pub name: String,
    pub declaration_path: Vec<String>,
    pub declaration_ordinal: usize,
    pub range: (usize, usize),
    pub body_range: (usize, usize),
    pub normal_form: Option<Value>,
    pub weight: usize,
    pub operations: usize,
    pub control_shape: bool,
    pub max_operation_chain: usize,
    pub ignored: Vec<(usize, usize)>,
    pub binder_names: Vec<String>,
    pub issues: Vec<SemanticIssue>,
}
fn text<'a>(s: &'a str, n: Node<'_>) -> &'a str {
    &s[n.byte_range()]
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn range(n: Node<'_>) -> (usize, usize) {
    (n.start_byte(), n.end_byte())
}
fn node(
    tag: &str,
    language: &str,
    variant: &str,
    mut fields: Vec<Value>,
    children: Vec<Value>,
) -> Value {
    let mut payload = vec![json!(language), json!(variant)];
    payload.append(&mut fields);
    json!([tag, payload, children])
}
#[derive(Clone)]
struct Binding {
    name: String,
    id: Value,
    ty: Value,
    initialized: bool,
}
struct Context<'a> {
    source: &'a str,
    language: &'a str,
    scopes: Vec<Vec<Binding>>,
    scope_ids: Vec<usize>,
    scope_table: Vec<Value>,
    binding_table: Vec<Value>,
    names: Vec<String>,
    issues: Vec<SemanticIssue>,
    ignored: Vec<(usize, usize)>,
    loops: Vec<usize>,
    next_loop: usize,
}
impl Context<'_> {
    fn issue(&mut self, n: Node<'_>, reason: &str, operation: &str) -> Value {
        self.issues.push(SemanticIssue {
            pattern: "duplication".into(),
            reason: reason.into(),
            range: range(n),
            operation: operation.into(),
        });
        Value::Null
    }
    fn runtime_type(&self) -> Value {
        json!(if self.language == "python" {
            "python-runtime-domain"
        } else {
            "ecmascript-runtime-domain"
        })
    }
    fn bind(
        &mut self,
        name: &str,
        kind: &str,
        ty: Value,
        initialized: bool,
        formal: Option<usize>,
    ) -> Value {
        let scope = *self.scope_ids.last().unwrap();
        let ordinal = self.scopes.last().unwrap().len();
        let id = if let Some(pos) = formal {
            json!(["formal", pos])
        } else {
            json!(["local", scope, ordinal])
        };
        self.scopes.last_mut().unwrap().push(Binding {
            name: name.into(),
            id: id.clone(),
            ty: ty.clone(),
            initialized,
        });
        self.binding_table.push(json!([
            id,
            scope,
            kind,
            ty,
            if self.language == "rust" {
                "scalar-copy"
            } else {
                "lexical-initialization"
            }
        ]));
        self.names.push(name.into());
        id
    }
    fn binding(&self, name: &str) -> Option<&Binding> {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.iter().find(|b| b.name == name))
    }
    fn literal(&mut self, n: Node<'_>) -> Value {
        let raw = text(self.source, n);
        let kind = n.kind();
        let ty = self.runtime_type();
        let value = match kind {
            "true" | "false" => json!(matches!(raw, "true" | "True")),
            "none" | "null" => Value::Null,
            "integer" => match raw.replace('_', "").parse::<i64>() {
                Ok(v) => json!(v),
                Err(_) => {
                    return self.issue(
                        n,
                        "DUP_UNSUPPORTED_CONSTRUCT",
                        "integer literal value/base exceeds implemented exact decoder",
                    )
                }
            },
            "number" | "float" => {
                let cleaned = raw.replace('_', "");
                let number = if let Some(hex) = cleaned
                    .strip_prefix("0x")
                    .or_else(|| cleaned.strip_prefix("0X"))
                {
                    u64::from_str_radix(hex, 16).ok().map(|v| v as f64)
                } else {
                    cleaned.parse::<f64>().ok()
                };
                match number {
                    Some(v) if v.is_finite() => json!(format!("{:016x}", v.to_bits())),
                    _ => {
                        return self.issue(
                            n,
                            "DUP_UNSUPPORTED_CONSTRUCT",
                            "exact numeric literal decoder",
                        )
                    }
                }
            }
            "string" => {
                // Exact scalar strings without escape/interpolation are a supported subset.
                if raw.contains('\\')
                    || raw.len() < 2
                    || !raw.starts_with(['\'', '"'])
                    || raw.starts_with("\"\"\"")
                    || raw.starts_with("'''")
                {
                    return self.issue(
                        n,
                        "DUP_UNSUPPORTED_CONSTRUCT",
                        "decoded escaped string contract",
                    );
                }
                if self.language == "javascript" || self.language == "typescript" {
                    json!(raw[1..raw.len() - 1].encode_utf16().collect::<Vec<_>>())
                } else {
                    json!(&raw[1..raw.len() - 1])
                }
            }
            _ => return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "literal kind"),
        };
        node(
            "literal",
            self.language,
            kind,
            vec![json!(kind), value, ty],
            vec![],
        )
    }
    fn expr(&mut self, n: Node<'_>, depth: usize) -> Value {
        if depth > super::MAX_AST_DEPTH {
            return self.issue(n, "DUP_CONTROL_UNRESOLVED", "normalization depth");
        }
        match n.kind() {
            "parenthesized_expression" => children(n)
                .first()
                .map_or(Value::Null, |x| self.expr(*x, depth + 1)),
            "identifier" => {
                let name = text(self.source, n);
                let Some(b) = self.binding(name).cloned() else {
                    return self.issue(
                        n,
                        "DUP_BINDING_UNRESOLVED",
                        "free/imported/external authority",
                    );
                };
                if !b.initialized {
                    return self.issue(n, "DUP_CONTROL_UNRESOLVED", "initialization state");
                }
                node(
                    "read",
                    self.language,
                    n.kind(),
                    vec![b.id, json!("value"), json!("initialized"), b.ty],
                    vec![],
                )
            }
            "number" | "integer" | "float" | "string" | "true" | "false" | "none" | "null" => {
                self.literal(n)
            }
            "binary_expression"
            | "binary_operator"
            | "boolean_operator"
            | "comparison_operator" => {
                let named = children(n);
                let (left, right) = if let (Some(l), Some(r)) = (
                    n.child_by_field_name("left"),
                    n.child_by_field_name("right"),
                ) {
                    (l, r)
                } else if named.len() == 2 {
                    (named[0], named[1])
                } else {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "comparison-chain mapping");
                };
                let operator = n
                    .child_by_field_name("operator")
                    .map(|op| text(self.source, op).to_string())
                    .unwrap_or_else(|| {
                        self.source[left.end_byte()..right.start_byte()]
                            .trim()
                            .to_string()
                    });
                if !matches!(
                    operator.as_str(),
                    "+" | "-"
                        | "*"
                        | "/"
                        | "%"
                        | "**"
                        | "//"
                        | "<"
                        | ">"
                        | "<="
                        | ">="
                        | "=="
                        | "!="
                        | "==="
                        | "!=="
                        | "and"
                        | "or"
                        | "&&"
                        | "||"
                        | "&"
                        | "|"
                        | "^"
                        | "<<"
                        | ">>"
                        | "is"
                        | "is not"
                ) {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "operator protocol");
                }
                let l = self.expr(left, depth + 1);
                let r = self.expr(right, depth + 1);
                let tag = if matches!(operator.as_str(), "and" | "or" | "&&" | "||") {
                    "short-circuit"
                } else {
                    "binary"
                };
                let domain = json!([
                    "result-of-operation",
                    self.language,
                    operator,
                    ["operand", 0],
                    ["operand", 1]
                ]);
                let effect = json!([
                    "exact-language-operation",
                    self.language,
                    operator,
                    ["left", "right"],
                    domain,
                    [
                        "normal",
                        "language-protocol-exception",
                        "language-protocol-divergence"
                    ]
                ]);
                node(tag, self.language, n.kind(), vec![effect], vec![l, r])
            }
            "unary_expression" | "unary_operator" | "not_operator" => {
                let Some(operand) = n
                    .child_by_field_name("argument")
                    .or_else(|| children(n).last().copied())
                else {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "unary operand");
                };
                let op = self.source[n.start_byte()..operand.start_byte()].trim();
                if !matches!(op, "-" | "+" | "!" | "not" | "~") {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "unary protocol");
                }
                let value = self.expr(operand, depth + 1);
                node(
                    "unary",
                    self.language,
                    n.kind(),
                    vec![json!([
                        "exact-language-operation",
                        self.language,
                        op,
                        ["operand"],
                        ["result", ["operand", 0]],
                        ["normal", "language-protocol-exception"]
                    ])],
                    vec![value],
                )
            }
            _ => self.issue(
                n,
                if matches!(n.kind(), "call" | "call_expression") {
                    "DUP_EFFECT_UNRESOLVED"
                } else {
                    "DUP_UNSUPPORTED_CONSTRUCT"
                },
                n.kind(),
            ),
        }
    }
    fn block(&mut self, n: Node<'_>, depth: usize, docstring: bool) -> Value {
        if depth > super::MAX_AST_DEPTH {
            return self.issue(n, "DUP_CONTROL_UNRESOLVED", "block depth");
        }
        let new_scope = self.language != "python";
        let scope = if new_scope {
            let id = self.scope_table.len();
            self.scope_table
                .push(json!([id, self.scope_ids.last(), "block"]));
            self.scopes.push(Vec::new());
            self.scope_ids.push(id);
            Some(id)
        } else {
            None
        };
        let mut statements = Vec::new();
        let mut real_statement = 0usize;
        for child in children(n).iter() {
            if child.kind() == "comment"
                || child.kind() == "pass_statement"
                || child.kind() == "empty_statement"
            {
                self.ignored.push(range(*child));
                continue;
            }
            if docstring
                && real_statement == 0
                && child.kind() == "expression_statement"
                && children(*child)
                    .first()
                    .is_some_and(|x| x.kind() == "string")
            {
                self.ignored.push(range(*child));
                continue;
            }
            if self.language != "python"
                && real_statement == 0
                && child.kind() == "expression_statement"
                && children(*child)
                    .first()
                    .is_some_and(|x| x.kind() == "string")
            {
                let literal = children(*child)[0];
                if text(self.source, literal).contains('\\') {
                    self.issue(
                        literal,
                        "DUP_UNSUPPORTED_CONSTRUCT",
                        "directive exact decoding",
                    );
                }
                statements.push(node(
                    "directive",
                    self.language,
                    child.kind(),
                    vec![json!(
                        &text(self.source, literal)[1..text(self.source, literal).len() - 1]
                    )],
                    vec![],
                ));
            } else {
                statements.push(self.statement(*child, depth + 1));
            }
            real_statement += 1;
        }
        if new_scope {
            self.scopes.pop();
            self.scope_ids.pop();
        }
        node(
            "block",
            self.language,
            n.kind(),
            vec![json!(scope)],
            statements,
        )
    }
    fn statement(&mut self, n: Node<'_>, depth: usize) -> Value {
        if depth > super::MAX_AST_DEPTH {
            return self.issue(n, "DUP_CONTROL_UNRESOLVED", "statement depth");
        }
        match n.kind() {
            "statement_block" | "block" => self.block(n, depth, false),
            "return_statement" | "return_expression" => {
                let child = children(n).first().copied();
                let expr = child.map_or(Value::Null, |x| self.expr(x, depth + 1));
                node(
                    "return",
                    self.language,
                    n.kind(),
                    vec![json!("explicit")],
                    vec![expr],
                )
            }
            "expression_statement" => {
                let Some(child) = children(n).first().copied() else {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "expression statement");
                };
                if matches!(
                    child.kind(),
                    "assignment" | "augmented_assignment" | "assignment_expression"
                ) {
                    self.statement(child, depth + 1)
                } else {
                    let expr = self.expr(child, depth + 1);
                    node(
                        "expr-statement",
                        self.language,
                        n.kind(),
                        vec![],
                        vec![expr],
                    )
                }
            }
            "lexical_declaration" => {
                let decls = children(n);
                if decls.len() != 1 {
                    return self.issue(
                        n,
                        "DUP_UNSUPPORTED_CONSTRUCT",
                        "multi-binding declaration mapping",
                    );
                }
                let Some(decl) = decls.first().copied() else {
                    return self.issue(n, "DUP_BINDING_UNRESOLVED", "declaration");
                };
                let Some(name) = decl
                    .child_by_field_name("name")
                    .filter(|x| x.kind() == "identifier")
                else {
                    return self.issue(decl, "DUP_BINDING_UNRESOLVED", "destructuring declaration");
                };
                let kind = n.child(0).map(|x| text(self.source, x)).unwrap_or("");
                let spelling = text(self.source, name);
                if self
                    .scopes
                    .last()
                    .unwrap()
                    .iter()
                    .any(|b| b.name == spelling)
                {
                    return self.issue(name, "DUP_BINDING_UNRESOLVED", "redeclaration");
                }
                let ty = self.runtime_type();
                let id = self.bind(spelling, kind, ty.clone(), false, None);
                let value = decl
                    .child_by_field_name("value")
                    .map(|x| self.expr(x, depth + 1));
                if let Some(b) = self.scopes.last_mut().unwrap().last_mut() {
                    b.initialized = value.is_some();
                }
                node(
                    "declare",
                    self.language,
                    decl.kind(),
                    vec![
                        id,
                        json!(kind),
                        ty,
                        json!(if kind == "const" {
                            "immutable"
                        } else {
                            "mutable"
                        }),
                    ],
                    value.into_iter().collect(),
                )
            }
            "assignment" | "assignment_expression" => {
                let Some(name) = n
                    .child_by_field_name("left")
                    .filter(|x| x.kind() == "identifier")
                else {
                    return self.issue(
                        n,
                        "DUP_BINDING_UNRESOLVED",
                        "nonidentifier mutation target",
                    );
                };
                let Some(value) = n.child_by_field_name("right") else {
                    return self.issue(n, "DUP_BINDING_UNRESOLVED", "assignment source");
                };
                let spelling = text(self.source, name);
                let id = if let Some(b) = self.binding(spelling) {
                    b.id.clone()
                } else if self.language == "python" {
                    self.bind(spelling, "assignment", self.runtime_type(), false, None)
                } else {
                    return self.issue(name, "DUP_BINDING_UNRESOLVED", "assignment target");
                };
                let expr = self.expr(value, depth + 1);
                for scope in self.scopes.iter_mut().rev() {
                    if let Some(b) = scope.iter_mut().find(|b| b.name == spelling) {
                        b.initialized = true;
                        break;
                    }
                }
                node(
                    "assign",
                    self.language,
                    n.kind(),
                    vec![id, json!("assignment")],
                    vec![expr],
                )
            }
            "if_statement" => {
                let condition = n.child_by_field_name("condition");
                let then = n.child_by_field_name("consequence");
                let alt = n.child_by_field_name("alternative");
                let (Some(condition), Some(then)) = (condition, then) else {
                    return self.issue(n, "DUP_CONTROL_UNRESOLVED", "if fields");
                };
                let cond = self.expr(condition, depth + 1);
                let before = self.scopes.clone();
                let yes = self.statement(then, depth + 1);
                let then_scopes = self.scopes.clone();
                self.scopes = before.clone();
                let no = alt.map_or(Value::Null, |a| {
                    if a.kind() == "else_clause" {
                        children(a)
                            .last()
                            .map_or(Value::Null, |x| self.statement(*x, depth + 1))
                    } else {
                        self.statement(a, depth + 1)
                    }
                });
                for (index, scope) in self.scopes.iter_mut().enumerate() {
                    for binding in scope {
                        binding.initialized &= then_scopes
                            .get(index)
                            .is_some_and(|s| s.iter().any(|b| b.id == binding.id && b.initialized));
                    }
                }
                node("if", self.language, n.kind(), vec![], vec![cond, yes, no])
            }
            "while_statement" => {
                let (Some(condition), Some(body)) = (
                    n.child_by_field_name("condition"),
                    n.child_by_field_name("body"),
                ) else {
                    return self.issue(n, "DUP_CONTROL_UNRESOLVED", "while fields");
                };
                if n.child_by_field_name("alternative").is_some() {
                    return self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", "while-else control");
                }
                let id = self.next_loop;
                self.next_loop += 1;
                self.loops.push(id);
                let cond = self.expr(condition, depth + 1);
                let before = self.scopes.clone();
                let body = self.statement(body, depth + 1);
                self.scopes = before;
                self.loops.pop();
                node(
                    "while",
                    self.language,
                    n.kind(),
                    vec![json!(id)],
                    vec![cond, body],
                )
            }
            "break_statement" | "continue_statement" => {
                if !children(n).is_empty() {
                    return self.issue(n, "DUP_CONTROL_UNRESOLVED", "labelled control target");
                }
                let Some(id) = self.loops.last().copied() else {
                    return self.issue(n, "DUP_CONTROL_UNRESOLVED", "loop target");
                };
                node(
                    if n.kind() == "break_statement" {
                        "break"
                    } else {
                        "continue"
                    },
                    self.language,
                    n.kind(),
                    vec![json!(id)],
                    vec![],
                )
            }
            _ => self.issue(n, "DUP_UNSUPPORTED_CONSTRUCT", n.kind()),
        }
    }
}
fn weight(n: &Value) -> (usize, usize, bool, usize, usize) {
    let tag = n[0].as_str().unwrap_or("");
    let operation = matches!(
        tag,
        "unary" | "binary" | "short-circuit" | "compare-chain" | "compound-assign"
    );
    let control = matches!(tag, "if" | "while");
    let mut total = usize::from(operation || control);
    let mut ops = usize::from(operation);
    let mut has_control = control;
    let mut maxchain = usize::from(operation);
    let mut current = usize::from(operation);
    for child in n[2].as_array().into_iter().flatten() {
        let (w, o, c, m, chain) = weight(child);
        total += w;
        ops += o;
        has_control |= c;
        maxchain = maxchain.max(m);
        if operation {
            current = current.max(1 + chain);
        }
    }
    maxchain = maxchain.max(current);
    (total, ops, has_control, maxchain, current)
}
/// Enumerate all function subjects, including explicit unsupported obligations.
pub(crate) fn collect(source: &str, language: &str, root: Node<'_>) -> Vec<NormalizedFunction> {
    let mut output = Vec::new();
    let mut pending = vec![(root, Vec::<String>::new(), 0usize)];
    while let Some((n, parents, depth)) = pending.pop() {
        if depth > super::MAX_AST_DEPTH {
            continue;
        }
        let is_fn = matches!(
            n.kind(),
            "function_declaration"
                | "function_definition"
                | "function_item"
                | "method_definition"
                | "arrow_function"
                | "function_expression"
                | "generator_function_declaration"
        );
        let mut next = parents.clone();
        if is_fn {
            let name = n
                .child_by_field_name("name")
                .map(|x| text(source, x))
                .unwrap_or("<anonymous>")
                .to_string();
            next.push(name.clone());
            let mut cx = Context {
                source,
                language,
                scopes: vec![Vec::new()],
                scope_ids: vec![0],
                scope_table: vec![json!([0, null, "function"])],
                binding_table: Vec::new(),
                names: Vec::new(),
                issues: Vec::new(),
                ignored: Vec::new(),
                loops: Vec::new(),
                next_loop: 0,
            };
            let mut ancestor = n.parent();
            let mut member_context = false;
            while let Some(parent) = ancestor {
                member_context |= matches!(
                    parent.kind(),
                    "class_definition" | "class_declaration" | "impl_item" | "trait_item"
                );
                ancestor = parent.parent();
            }
            let ordinary = !member_context
                && parents.is_empty()
                && matches!(
                    n.kind(),
                    "function_declaration" | "function_definition" | "function_item"
                )
                && !text(source, n).trim_start().starts_with("async ")
                && n.parent()
                    .is_none_or(|p| p.kind() != "decorated_definition");
            if !ordinary {
                cx.issue(
                    n,
                    "DUP_UNSUPPORTED_SUBJECT",
                    "ordinary named function required",
                );
            }
            if ordinary && (language == "rust" || language == "typescript") {
                cx.issue(
                    n,
                    "DUP_TYPE_UNRESOLVED",
                    "native static type/inference/ownership normal-form mapping",
                );
            }
            let mut slots = Vec::new();
            if let Some(params) = n.child_by_field_name("parameters") {
                let named = children(params);
                let positional_only = named
                    .iter()
                    .position(|p| p.kind() == "positional_separator");
                for param in named {
                    if param.kind() == "positional_separator" {
                        continue;
                    }
                    if param.kind() != "identifier" {
                        cx.issue(
                            param,
                            "DUP_UNSUPPORTED_CONSTRUCT",
                            "simple parameter signature required",
                        );
                        continue;
                    }
                    let position = slots.len();
                    let name = text(source, param);
                    let ty = cx.runtime_type();
                    let id = cx.bind(name, "parameter", ty.clone(), true, Some(position));
                    let visible = if language == "python"
                        && positional_only.is_none_or(|end| position >= end)
                    {
                        json!(name)
                    } else {
                        Value::Null
                    };
                    slots.push(json!([
                        if visible.is_null() {
                            "positional"
                        } else {
                            "positional-or-keyword"
                        },
                        id,
                        visible,
                        ty,
                        "value"
                    ]));
                }
            } else {
                cx.issue(n, "DUP_BINDING_UNRESOLVED", "signature parameters");
            }
            let signature = json!([
                "ordinary",
                if language == "python" {
                    "python-call"
                } else {
                    "ecmascript-call"
                },
                slots,
                cx.runtime_type()
            ]);
            let body = n.child_by_field_name("body");
            let body_range = body.map(range).unwrap_or(range(n));
            let body_tree = body.map_or(Value::Null, |b| cx.block(b, 0, language == "python"));
            let tree = node(
                "function",
                language,
                n.kind(),
                vec![
                    json!("ordinary"),
                    signature,
                    json!(["environment-from-explicit-policy"]),
                ],
                vec![body_tree],
            );
            let (w, ops, control, maxchain, _) = weight(&tree);
            let (types, effects, control_edges) = semantic_tables(&tree);
            let normal = json!([
                "flowspec.duplication.normalform.v1",
                tree,
                cx.scope_table,
                cx.binding_table,
                types,
                effects,
                control_edges
            ]);
            if language == "rust" || language == "typescript" {
                cx.issues.retain(|issue| {
                    issue.reason == "DUP_TYPE_UNRESOLVED"
                        || issue.reason == "DUP_UNSUPPORTED_SUBJECT"
                });
            }
            let declaration_ordinal = output
                .iter()
                .filter(|f: &&NormalizedFunction| f.declaration_path == next)
                .count();
            output.push(NormalizedFunction {
                name,
                declaration_path: next.clone(),
                declaration_ordinal,
                range: range(n),
                body_range,
                normal_form: cx.issues.is_empty().then_some(normal),
                weight: w,
                operations: ops,
                control_shape: control,
                max_operation_chain: maxchain,
                ignored: cx.ignored,
                binder_names: cx.names,
                issues: cx.issues,
            });
        }
        let mut kids = children(n);
        kids.reverse();
        for child in kids {
            pending.push((child, next.clone(), depth + 1));
        }
    }
    output
}

/// Ordered exact syntax/control protocols. Evaluation edges distinguish entering
/// a term from receiving its value; no execution order is inferred from a set.
fn semantic_tables(tree: &Value) -> (Value, Value, Value) {
    fn visit(
        n: &Value,
        address: Vec<usize>,
        next: Value,
        loops: &std::collections::HashMap<u64, (Value, Value)>,
        types: &mut Vec<Value>,
        effects: &mut Vec<Value>,
        control: &mut Vec<Value>,
    ) {
        let Some(tag) = n[0].as_str() else { return };
        let here = json!(address);
        let children = n[2].as_array().cloned().unwrap_or_default();
        let addr = |index: usize| {
            let mut a = address.clone();
            a.push(index);
            json!(a)
        };
        let mut edge = |kind: &str, target: Value| control.push(json!([here, kind, target]));
        match tag {
            "literal" => types.push(json!([here, n[1][4]])),
            "read" => types.push(json!([here, n[1][5]])),
            "declare" => types.push(json!([here, n[1][4]])),
            "function" => types.push(json!([here, n[1][3]])),
            "unary" | "binary" | "short-circuit" => effects.push(json!([here, n[1][2]])),
            _ => {}
        }
        match tag {
            "return" => {
                if !children[0].is_null() {
                    edge("evaluate-return-value", addr(0));
                }
                edge("return", json!("function-normal-exit"));
            }
            "if" => {
                edge("evaluate-condition", addr(0));
                edge("condition-true", addr(1));
                edge(
                    "condition-false",
                    if children[2].is_null() {
                        next.clone()
                    } else {
                        addr(2)
                    },
                );
            }
            "while" => {
                edge("evaluate-condition", addr(0));
                edge("condition-true", addr(1));
                edge("condition-false", next.clone());
            }
            "break" | "continue" => {
                if let Some((condition, exit)) = n[1][2].as_u64().and_then(|id| loops.get(&id)) {
                    edge(
                        tag,
                        if tag == "break" {
                            exit.clone()
                        } else {
                            condition.clone()
                        },
                    );
                }
            }
            "short-circuit" => {
                edge("evaluate-left", addr(0));
                edge("evaluate-right-if-required-by-protocol", addr(1));
                edge("short-circuit-normal", next.clone());
                edge("right-result-normal", next.clone());
                edge("protocol-exception", json!("function-exception-exit"));
                edge("protocol-divergence", json!("function-divergence"));
            }
            "binary" | "unary" => {
                if !children.is_empty() {
                    edge("evaluate-first-operand", addr(0));
                }
                edge("operation-normal", next.clone());
                edge("protocol-exception", json!("function-exception-exit"));
                edge("protocol-divergence", json!("function-divergence"));
            }
            "block" | "function" => {
                edge(
                    "enter",
                    if children.is_empty() {
                        next.clone()
                    } else {
                        addr(0)
                    },
                );
            }
            _ => {
                if !children.is_empty() && !children[0].is_null() {
                    edge("evaluate", addr(0));
                }
                edge("normal", next.clone());
            }
        }
        let mut nested_loops = loops.clone();
        if tag == "while" {
            if let Some(id) = n[1][2].as_u64() {
                nested_loops.insert(id, (addr(0), next.clone()));
            }
        }
        for (i, child) in children.iter().enumerate() {
            if child.is_null() {
                continue;
            }
            let continuation = match tag {
                "block" => {
                    if i + 1 < children.len() {
                        addr(i + 1)
                    } else {
                        next.clone()
                    }
                }
                "function" => json!("function-normal-exit"),
                "if" => {
                    if i == 0 {
                        here.clone()
                    } else {
                        next.clone()
                    }
                }
                "while" => {
                    if i == 0 {
                        here.clone()
                    } else {
                        addr(0)
                    }
                }
                "return" => here.clone(),
                "binary" | "unary" => {
                    if i + 1 < children.len() {
                        addr(i + 1)
                    } else {
                        here.clone()
                    }
                }
                "short-circuit" => here.clone(),
                _ => here.clone(),
            };
            let mut child_address = address.clone();
            child_address.push(i);
            visit(
                child,
                child_address,
                continuation,
                &nested_loops,
                types,
                effects,
                control,
            );
        }
    }
    let (mut types, mut effects, mut control) = (Vec::new(), Vec::new(), Vec::new());
    visit(
        tree,
        Vec::new(),
        json!("function-normal-exit"),
        &Default::default(),
        &mut types,
        &mut effects,
        &mut control,
    );
    // Fixed numeric child-address preorder; storage sorting cannot reorder eval.
    control.sort_by(|a, b| {
        let a_addr: Vec<u64> = a[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        let b_addr: Vec<u64> = b[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        a_addr
            .cmp(&b_addr)
            .then_with(|| a[1].as_str().cmp(&b[1].as_str()))
            .then_with(|| canonical_bytes(&a[2]).cmp(&canonical_bytes(&b[2])))
    });
    (json!(types), json!(effects), json!(control))
}
/// Rule section8 JSON: ordered arrays, exact integers, lowercase four-digit
/// control escapes, and unescaped non-ASCII. Hashing never replaces byte equality.
pub fn canonical_bytes(value: &Value) -> Vec<u8> {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Number(n) => out.push_str(&n.to_string()),
            Value::String(s) => {
                out.push('"');
                for c in s.chars() {
                    match c {
                        '"' => out.push_str("\\\""),
                        '\\' => out.push_str("\\\\"),
                        c if (c as u32) < 32 => out.push_str(&format!("\\u{:04x}", c as u32)),
                        c => out.push(c),
                    }
                }
                out.push('"');
            }
            Value::Array(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Object(map) => {
                out.push('{');
                for (i, (key, val)) in map.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write(&json!(key), out);
                    out.push(':');
                    write(val, out);
                }
                out.push('}');
            }
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out.into_bytes()
}
