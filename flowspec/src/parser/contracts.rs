//! Native declaration and literal operand facts. An annotation describes the
//! contract, never the dynamic type of an identifier argument.
use serde::{Deserialize, Serialize};
use tree_sitter::Node;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureFact {
    pub name: String,
    pub range: (usize, usize),
    pub parameter_types: Vec<Option<String>>,
    pub return_type: Option<String>,
    pub literal_returns: Vec<((usize, usize), String)>,
    pub unsupported_parameters: bool,
    pub literal_leaf: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiteralCall {
    pub range: (usize, usize),
    pub arguments: Vec<Option<String>>,
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn span(n: Node<'_>) -> (usize, usize) {
    (n.start_byte(), n.end_byte())
}
fn primitive(s: &str) -> Option<String> {
    match s.trim().trim_start_matches(':').trim() {
        "bool" | "boolean" => Some("boolean".into()),
        "str" | "string" | "&str" => Some("string".into()),
        "number" => Some("number".into()),
        _ => None,
    }
}
fn literal(n: Node<'_>, s: &str) -> Option<String> {
    match n.kind() {
        "true" | "false" | "boolean_literal" => Some("boolean".into()),
        "number" => Some("number".into()),
        "string" | "string_literal"
            if !s[n.byte_range()].starts_with(['b', 'B'])
                && !children(n).iter().any(|c| c.kind() == "interpolation") =>
        {
            Some("string".into())
        }
        "identifier" if matches!(&s[n.byte_range()], "True" | "False") => Some("boolean".into()),
        _ => None,
    }
}
// JSDoc belongs to the immediately adjacent declaration, including an export
// wrapper. Intervening statements or comments cannot donate an annotation.
fn jsdoc_return(n: Node<'_>, source: &str) -> Option<String> {
    if n.kind() != "function_declaration" {
        return None;
    }
    let declaration = n
        .parent()
        .filter(|p| p.kind() == "export_statement")
        .unwrap_or(n);
    let comment = declaration.prev_named_sibling()?;
    if comment.kind() != "comment"
        || !source[comment.end_byte()..declaration.start_byte()]
            .trim()
            .is_empty()
    {
        return None;
    }
    let text = &source[comment.byte_range()];
    if !text.starts_with("/**") || !text.ends_with("*/") {
        return None;
    }
    let mut found = None;
    for tag in ["@returns", "@return"] {
        if let Some(at) = text.find(tag) {
            let rest = text[at + tag.len()..].trim_start();
            // @returns must not also be interpreted as @return.
            if !rest.starts_with('{') {
                continue;
            }
            let end = rest.find('}')?;
            let ty = primitive(&rest[1..end])?;
            if found.is_some() {
                return None;
            }
            found = Some(ty);
        }
    }
    found
}
pub(crate) fn collect(s: &str, root: Node<'_>) -> (Vec<SignatureFact>, Vec<LiteralCall>) {
    let mut signatures = Vec::new();
    let mut calls = Vec::new();
    let mut pending = vec![(root, 0)];
    while let Some((n, d)) = pending.pop() {
        if d > super::MAX_AST_DEPTH {
            continue;
        }
        if matches!(
            n.kind(),
            "function_definition" | "function_declaration" | "function_item"
        ) {
            if let (Some(name), Some(params)) = (
                n.child_by_field_name("name"),
                n.child_by_field_name("parameters"),
            ) {
                let mut types = Vec::new();
                let mut unsupported = false;
                for p in children(params) {
                    if p.kind() == "comment" {
                        continue;
                    }
                    let t = p.child_by_field_name("type").map(|t| &s[t.byte_range()]);
                    let simple = matches!(
                        p.kind(),
                        "identifier" | "typed_parameter" | "required_parameter" | "parameter"
                    ) && p.child_by_field_name("value").is_none();
                    unsupported |= !simple;
                    types.push(t.and_then(primitive));
                }
                let ret = n
                    .child_by_field_name("return_type")
                    .and_then(|t| primitive(&s[t.byte_range()]))
                    .or_else(|| jsdoc_return(n, s));
                let mut returns = Vec::new();
                let mut literal_leaf = false;
                if let Some(body) = n.child_by_field_name("body") {
                    let statements: Vec<_> = children(body)
                        .into_iter()
                        .filter(|n| n.kind() != "comment")
                        .collect();
                    if statements.len() == 1 {
                        let value = statements[0];
                        let value =
                            if matches!(value.kind(), "return_statement" | "return_expression") {
                                value.named_child(0)
                            } else {
                                Some(value)
                            };
                        literal_leaf = value.is_some_and(|v| literal(v, s).is_some());
                    }
                    let mut stack = children(body);
                    while let Some(x) = stack.pop() {
                        if matches!(
                            x.kind(),
                            "function_definition"
                                | "function_declaration"
                                | "function_item"
                                | "arrow_function"
                        ) {
                            continue;
                        }
                        if matches!(x.kind(), "return_statement" | "return_expression") {
                            if let Some(v) = x.named_child(0) {
                                if let Some(t) = literal(v, s) {
                                    returns.push((span(v), t));
                                }
                            }
                        } else {
                            stack.extend(children(x));
                        }
                    }
                    if let Some(tail) = children(body).last().copied() {
                        if let Some(t) = literal(tail, s) {
                            returns.push((span(tail), t));
                        }
                    }
                }
                signatures.push(SignatureFact {
                    name: s[name.byte_range()].into(),
                    range: span(n),
                    parameter_types: types,
                    return_type: ret,
                    literal_returns: returns,
                    unsupported_parameters: unsupported,
                    literal_leaf,
                });
            }
        }
        if matches!(n.kind(), "call" | "call_expression") {
            if let Some(args) = n.child_by_field_name("arguments") {
                calls.push(LiteralCall {
                    range: span(n),
                    arguments: children(args)
                        .into_iter()
                        .filter(|a| a.kind() != "comment")
                        .map(|a| literal(a, s))
                        .collect(),
                });
            }
        }
        pending.extend(children(n).into_iter().map(|n| (n, d + 1)));
    }
    (signatures, calls)
}
