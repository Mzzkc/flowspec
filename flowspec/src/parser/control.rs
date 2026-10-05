//! Original-byte normal-return CFGs for explicit invocation policies. Exceptional
//! and divergent paths are outside this contract, never erased into all-exit proof.
use serde::{Deserialize, Serialize};
use tree_sitter::Node;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NormalControl {
    pub name: String,
    pub range: (usize, usize),
    pub paths: Vec<Vec<(usize, usize)>>,
    pub unresolved: Vec<(usize, usize)>,
    #[serde(default)]
    pub all_paths: Vec<Vec<(usize, usize)>>,
    #[serde(default)]
    pub exceptional_paths: usize,
}
#[derive(Clone, Default)]
struct Path {
    calls: Vec<(usize, usize)>,
    returned: bool,
    exceptional: bool,
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn span(n: Node<'_>) -> (usize, usize) {
    (n.start_byte(), n.end_byte())
}
fn expression(n: Node<'_>, path: &mut Path, unknown: &mut Vec<(usize, usize)>, depth: usize) {
    if depth > super::MAX_AST_DEPTH {
        unknown.push(span(n));
        return;
    }
    match n.kind() {
        "identifier" | "integer" | "float" | "integer_literal" | "float_literal" | "number"
        | "true" | "false" | "boolean_literal" | "none" | "null" | "type_identifier" | "string"
        | "string_literal" => {}
        "call" | "call_expression" => {
            let target = n.child_by_field_name("function");
            if !target.is_some_and(|t| matches!(t.kind(), "identifier" | "scoped_identifier")) {
                unknown.push(span(n));
            }
            if let Some(args) = n.child_by_field_name("arguments") {
                for arg in children(args) {
                    expression(arg, path, unknown, depth + 1);
                }
            }
            path.calls.push(span(n));
        }
        "parenthesized_expression"
        | "binary_expression"
        | "binary_operator"
        | "unary_expression"
        | "unary_operator"
        | "comparison_operator" => {
            // Short-circuit expressions require their own path split; refusing
            // avoids claiming RHS calls are mandatory on every return path.
            let short = children(n)
                .iter()
                .any(|c| matches!(c.kind(), "boolean_operator"));
            let op = n.child_by_field_name("operator");
            if short || op.is_some_and(|o| matches!(o.kind(), "&&" | "||" | "and" | "or")) {
                unknown.push(span(n));
            }
            for child in children(n) {
                expression(child, path, unknown, depth + 1);
            }
        }
        _ => unknown.push(span(n)),
    }
}
fn statement(
    source: &str,
    n: Node<'_>,
    paths: Vec<Path>,
    unknown: &mut Vec<(usize, usize)>,
    depth: usize,
) -> Vec<Path> {
    if depth > super::MAX_AST_DEPTH || paths.len() > 256 {
        unknown.push(span(n));
        return paths;
    }
    match n.kind() {
        "block" | "statement_block" => {
            let mut result = paths;
            for child in children(n) {
                result = statement(source, child, result, unknown, depth + 1);
            }
            result
        }
        "expression_statement"
            if n.named_child(0)
                .is_some_and(|c| matches!(c.kind(), "if_expression" | "return_expression")) =>
        {
            statement(source, n.named_child(0).unwrap(), paths, unknown, depth + 1)
        }
        "if_statement" | "if_expression" => {
            let mut result = Vec::new();
            for mut path in paths {
                if path.returned {
                    result.push(path);
                    continue;
                }
                if let Some(condition) = n.child_by_field_name("condition") {
                    expression(condition, &mut path, unknown, depth + 1);
                }
                let condition = n.child_by_field_name("condition");
                let known = condition.and_then(|c| {
                    match source[c.byte_range()]
                        .trim()
                        .trim_start_matches('(')
                        .trim_end_matches(')')
                        .trim()
                    {
                        "true" | "True" => Some(true),
                        "false" | "False" => Some(false),
                        _ => None,
                    }
                });
                let yes = n.child_by_field_name("consequence");
                let no = n.child_by_field_name("alternative");
                if known != Some(false) {
                    if let Some(yes) = yes {
                        result.extend(statement(
                            source,
                            yes,
                            vec![path.clone()],
                            unknown,
                            depth + 1,
                        ));
                    } else {
                        unknown.push(span(n));
                    }
                }
                if known != Some(true) {
                    if let Some(no) = no {
                        result.extend(statement(source, no, vec![path], unknown, depth + 1));
                    } else {
                        result.push(path);
                    }
                }
            }
            result
        }
        "else_clause" => {
            if let Some(body) = n.child_by_field_name("body").or_else(|| n.named_child(0)) {
                statement(source, body, paths, unknown, depth + 1)
            } else {
                unknown.push(span(n));
                paths
            }
        }
        "throw_statement" | "raise_statement" => paths
            .into_iter()
            .map(|mut p| {
                if !p.returned {
                    for child in children(n) {
                        expression(child, &mut p, unknown, depth + 1);
                    }
                    p.returned = true;
                    p.exceptional = true;
                }
                p
            })
            .collect(),
        "return_statement" | "return_expression" => paths
            .into_iter()
            .map(|mut p| {
                if !p.returned {
                    for child in children(n) {
                        expression(child, &mut p, unknown, depth + 1);
                    }
                    p.returned = true;
                }
                p
            })
            .collect(),
        "expression_statement" => paths
            .into_iter()
            .map(|mut p| {
                if !p.returned {
                    for child in children(n) {
                        expression(child, &mut p, unknown, depth + 1);
                    }
                }
                p
            })
            .collect(),
        "pass_statement" | "empty_statement" => paths,
        // A Rust scalar tail expression is a normal-return path. Loop/resource/
        // exceptional syntax is refused below instead of truncated into a CFG.
        "identifier" | "integer_literal" | "boolean_literal" | "call_expression"
        | "binary_expression" => paths
            .into_iter()
            .map(|mut p| {
                if !p.returned {
                    expression(n, &mut p, unknown, depth + 1);
                }
                p
            })
            .collect(),
        _ => {
            unknown.push(span(n));
            paths
        }
    }
}
pub(crate) fn collect(source: &str, root: Node<'_>) -> Vec<NormalControl> {
    let mut pending = vec![(root, 0)];
    let mut result = Vec::new();
    while let Some((n, depth)) = pending.pop() {
        if depth > super::MAX_AST_DEPTH {
            continue;
        }
        if matches!(
            n.kind(),
            "function_definition" | "function_declaration" | "function_item"
        ) {
            if let (Some(name), Some(body)) =
                (n.child_by_field_name("name"), n.child_by_field_name("body"))
            {
                let mut unknown = Vec::new();
                let paths = statement(source, body, vec![Path::default()], &mut unknown, 0);
                // Names are supplied by the grammar byte span, not a text search.
                result.push(NormalControl {
                    name: source[name.byte_range()].to_string(),
                    range: span(n),
                    exceptional_paths: paths.iter().filter(|p| p.exceptional).count(),
                    all_paths: paths.iter().map(|p| p.calls.clone()).collect(),
                    paths: paths
                        .into_iter()
                        .filter(|p| !p.exceptional)
                        .map(|p| p.calls)
                        .collect(),
                    unresolved: unknown,
                });
            }
        } else {
            pending.extend(children(n).into_iter().rev().map(|c| (c, depth + 1)));
        }
    }
    result
}
