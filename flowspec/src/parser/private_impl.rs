//! Closed private-owner implementation facts for a finite Rust subset. Proof
//! requires an actual nominal declaration, explicit inherent impl and no owner
//! use/escape/registration in the complete original-byte single-file scope.
use serde::{Deserialize, Serialize};
use tree_sitter::Node;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivateImplementation {
    pub qualified_name: String,
    pub owner: String,
    pub range: (usize, usize),
    pub owner_range: (usize, usize),
    pub implementation_range: (usize, usize),
    pub closure: bool,
}
fn children(n: Node<'_>) -> Vec<Node<'_>> {
    let mut c = n.walk();
    n.named_children(&mut c).collect()
}
fn span(n: Node<'_>) -> (usize, usize) {
    (n.start_byte(), n.end_byte())
}
pub(crate) fn collect(source: &str, file: &str, root: Node<'_>) -> Vec<PrivateImplementation> {
    let mut all = Vec::new();
    let mut stack = vec![(root, 0)];
    let mut registry_closed = true;
    while let Some((n, d)) = stack.pop() {
        if d > super::MAX_AST_DEPTH {
            registry_closed = false;
            continue;
        }
        if matches!(
            n.kind(),
            "macro_invocation"
                | "attribute_item"
                | "inner_attribute_item"
                | "use_declaration"
                | "mod_item"
        ) {
            registry_closed = false;
        }
        all.push(n);
        stack.extend(children(n).into_iter().map(|n| (n, d + 1)));
    }
    let mut result = Vec::new();
    for declaration in children(root)
        .into_iter()
        .filter(|n| n.kind() == "struct_item")
    {
        if children(declaration)
            .iter()
            .any(|n| n.kind() == "visibility_modifier")
        {
            continue;
        }
        let Some(name) = declaration.child_by_field_name("name") else {
            continue;
        };
        let owner = &source[name.byte_range()];
        let impls: Vec<_> = children(root)
            .into_iter()
            .filter(|n| {
                n.kind() == "impl_item"
                    && n.child_by_field_name("trait").is_none()
                    && n.child_by_field_name("type").is_some_and(|t| {
                        t.kind() == "type_identifier" && &source[t.byte_range()] == owner
                    })
            })
            .collect();
        let allowed: Vec<_> = std::iter::once(name.start_byte())
            .chain(
                impls
                    .iter()
                    .filter_map(|i| i.child_by_field_name("type").map(|t| t.start_byte())),
            )
            .collect();
        let escapes = all.iter().any(|n| {
            matches!(n.kind(), "identifier" | "type_identifier")
                && (&source[n.byte_range()] == owner || &source[n.byte_range()] == "Self")
                && !allowed.contains(&n.start_byte())
        });
        for implementation in impls {
            let Some(body) = implementation.child_by_field_name("body") else {
                continue;
            };
            for method in children(body)
                .into_iter()
                .filter(|n| n.kind() == "function_item")
            {
                let Some(m) = method.child_by_field_name("name") else {
                    continue;
                };
                result.push(PrivateImplementation {
                    qualified_name: format!("{file}::{owner}::{}", &source[m.byte_range()]),
                    owner: format!("{file}::{owner}"),
                    range: span(method),
                    owner_range: span(declaration),
                    implementation_range: span(implementation),
                    closure: registry_closed && !escapes,
                });
            }
        }
    }
    result
}
