//! Type operations tied to resolved references and exact source bindings.
//! Annotation reads establish declarations; only executable sites create or consume values.
use crate::{Graph, manifest::types::{TypeFlowEntry,TypeTransformation}};
use crate::parser::ir::{SymbolId,SymbolKind,ReferenceKind,ResolutionStatus,Location,EdgeKind};
use std::{collections::{BTreeMap,HashSet},path::Path};

fn concrete_type(graph:&Graph,mut id:SymbolId)->Option<SymbolId> {
    let mut seen=HashSet::new();
    while seen.insert(id) {
        let symbol=graph.get_symbol(id)?;
        if matches!(symbol.kind,SymbolKind::Class|SymbolKind::Struct|SymbolKind::Interface) && !symbol.annotations.iter().any(|a|a=="import") { return Some(id); }
        if !symbol.annotations.iter().any(|a|a=="import") {return None;}
        let targets:HashSet<_>=graph.edges_from(id).iter().filter(|e|e.kind==EdgeKind::References).map(|e|e.target).collect();
        if targets.len()!=1 {return None;} id=*targets.iter().next()?;
    }
    None
}
fn within(node:tree_sitter::Node<'_>,location:&Location)->bool {
    let a=node.start_position();let b=node.end_position();
    (location.line,location.column)>=(a.row as u32+1,a.column as u32+1) &&
    (location.end_line,location.end_column)<=(b.row as u32+1,b.column as u32+1)
}
fn text<'a>(node:tree_sitter::Node<'_>,source:&'a str)->&'a str { source.get(node.byte_range()).unwrap_or("") }
fn annotation_type(graph:&Graph,refs:&[&crate::parser::ir::Reference],node:tree_sitter::Node<'_>)->Option<SymbolId> {
    let found:HashSet<_>=refs.iter().filter(|r|r.kind==ReferenceKind::Annotation && r.resolution==ResolutionStatus::Resolved && within(node,&r.location)).filter_map(|r|concrete_type(graph,r.to)).collect();
    if found.len()==1 {found.iter().next().copied()} else {None}
}
fn bound_call_type(graph:&Graph,reference:&crate::parser::ir::Reference)->Option<SymbolId> {
    if reference.kind!=ReferenceKind::Call {return None;}
    match &reference.resolution {
        ResolutionStatus::Resolved=>concrete_type(graph,reference.to),
        // Old population preserves this parser marker on concretely bound local edges.
        // Accept only the exact unique local declaration, never unresolved/dynamic markers.
        ResolutionStatus::Partial(info)=> {
            let name=info.strip_prefix("call:")?;
            let target=concrete_type(graph,reference.to)?;let symbol=graph.get_symbol(target)?;
            if symbol.location.file!=reference.location.file || symbol.name!=name {return None;}
            if graph.all_symbols().filter(|(_,s)|s.location.file==reference.location.file && s.name==name).count()!=1 {return None;}
            Some(target)
        }
        _=>None,
    }
}
fn record<'a>(records:&'a mut BTreeMap<String,TypeFlowEntry>,name:&str)->&'a mut TypeFlowEntry {
    records.entry(name.into()).or_insert_with(||TypeFlowEntry {type_name:name.into(),created_at:vec![],transformed_to:vec![],consumed_by:vec![],lifetime:"unknown".into()})
}
/// All source text is the same captured text used to build the current graph.
/// No source reread, filename/name matching, or Annotation-to-Value promotion occurs.
pub fn derive_type_flows<'a>(graph:&Graph,sources:impl IntoIterator<Item=(&'a Path,&'a str)>)->Vec<TypeFlowEntry> {
    let mut records=BTreeMap::new();
    for (path,source) in sources {
        let refs=graph.references_from_file(path);
        let language:tree_sitter::Language=match path.extension().and_then(|e|e.to_str()) {
            Some("py")=>tree_sitter_python::LANGUAGE.into(),
            Some("rs")=>tree_sitter_rust::LANGUAGE.into(),
            Some("js"|"jsx"|"mjs"|"cjs")=>tree_sitter_javascript::LANGUAGE.into(),
            // Native TypeScript is the structured adapter owner's seam. Never reparse as JavaScript.
            _=>continue,
        };
        let mut parser=tree_sitter::Parser::new();if parser.set_language(&language).is_err(){continue;}
        let Some(tree)=parser.parse(source,None) else {continue;};
        if tree.root_node().has_error(){continue;}
        let mut pending=vec![(tree.root_node(),0usize)];
        while let Some((node,depth))=pending.pop() {
            if depth>crate::parser::MAX_AST_DEPTH {continue;}
            let mut cursor=node.walk();pending.extend(node.named_children(&mut cursor).map(|n|(n,depth+1)));
            if !matches!(node.kind(),"function_definition"|"function_item"|"function_declaration"|"method_definition") {continue;}
            let Some((owner_id,owner))=graph.all_symbols().find(|(_,s)|s.location.file==path && matches!(s.kind,SymbolKind::Function|SymbolKind::Method) && s.location.line==node.start_position().row as u32+1 && s.location.column==node.start_position().column as u32+1) else {continue;};
            let Some(body)=node.child_by_field_name("body") else {continue;};
            let Some(params)=node.child_by_field_name("parameters") else {continue;};
            let mut parameters=Vec::new();let mut cursor=params.walk();
            for parameter in params.named_children(&mut cursor) {
                let Some(typ)=parameter.child_by_field_name("type") else {continue;};
                let Some(type_id)=annotation_type(graph,&refs,typ) else {continue;};
                let name_node=parameter.child_by_field_name("pattern").or_else(||parameter.child_by_field_name("name")).or_else(||parameter.named_child(0));
                if let Some(name)=name_node.filter(|n|n.kind()=="identifier") {parameters.push((text(name,source).to_string(),type_id));}
            }
            let mut body_nodes=vec![body];
            let mut rebound=HashSet::new();
            for parameter in &parameters {rebound.insert(parameter.0.clone());}
            let mut cursor=params.walk();
            for parameter in params.named_children(&mut cursor) {
                let name=parameter.child_by_field_name("pattern").or_else(||parameter.child_by_field_name("name")).or_else(||parameter.named_child(0)).unwrap_or(parameter);
                if name.kind()=="identifier" {rebound.insert(text(name,source).to_string());}
            }
            while let Some(operation)=body_nodes.pop() {
                if operation!=body && matches!(operation.kind(),"function_definition"|"function_item"|"lambda"|"class_definition"|"function_declaration") {continue;}
                let mut cursor=operation.walk();body_nodes.extend(operation.named_children(&mut cursor));
                if matches!(operation.kind(),"assignment"|"augmented_assignment"|"let_declaration") {
                    if let Some(left)=operation.child_by_field_name("left").or_else(||operation.child_by_field_name("pattern")) {rebound.insert(text(left,source).into());}
                }
            }
            for reference in refs.iter().filter(|r|r.from==owner_id && within(body,&r.location)) {
                if let Some(target)=bound_call_type(graph,reference) {
                    if let Some(typ)=graph.get_symbol(target).filter(|s|!rebound.contains(&s.name)) {record(&mut records,&typ.qualified_name).created_at.push(owner.qualified_name.clone());}
                }
            }
            let return_type=node.child_by_field_name("return_type").and_then(|n|annotation_type(graph,&refs,n));
            for (name,input_type) in parameters {
                let mut executable=vec![body];let mut used=false;let mut shadowed=false;let mut transformations=Vec::new();
                while let Some(operation)=executable.pop() {
                    if operation!=body && matches!(operation.kind(),"function_definition"|"function_item"|"lambda"|"class_definition"|"function_declaration") {continue;}
                    let mut cursor=operation.walk();executable.extend(operation.named_children(&mut cursor));
                    if matches!(operation.kind(),"assignment"|"augmented_assignment"|"let_declaration") {
                        let left=operation.child_by_field_name("left").or_else(||operation.child_by_field_name("pattern"));
                        if left.is_some_and(|n|text(n,source)==name) {shadowed=true;}
                    }
                    if operation.kind()=="identifier" && text(operation,source)==name {used=true;}
                    if matches!(operation.kind(),"call"|"call_expression") {
                        let Some(arguments)=operation.child_by_field_name("arguments") else {continue;};
                        let mut cursor=arguments.walk();
                        if !arguments.named_children(&mut cursor).any(|n|n.kind()=="identifier"&&text(n,source)==name) {continue;}
                        let call=refs.iter().find(|r|r.from==owner_id && r.kind==ReferenceKind::Call && r.location.line==operation.start_position().row as u32+1 && r.location.column==operation.start_position().column as u32+1);
                        if operation.parent().is_none_or(|parent| !matches!(parent.kind(),"return_statement"|"return_expression")) {continue;}
                        if let Some(target)=call.and_then(|r|bound_call_type(graph,r)).filter(|target|Some(*target)==return_type) {
                            if let Some(typ)=graph.get_symbol(target) {transformations.push(TypeTransformation {target_type:typ.qualified_name.clone(),via:owner.qualified_name.clone()});}
                        }
                    }
                }
                if used && !shadowed {
                    let Some(typ)=graph.get_symbol(input_type) else {continue;};
                    let flow=record(&mut records,&typ.qualified_name);flow.consumed_by.push(owner.qualified_name.clone());flow.transformed_to.extend(transformations);
                }
            }
        }
    }
    for flow in records.values_mut() {
        flow.created_at.sort();flow.created_at.dedup();flow.consumed_by.sort();flow.consumed_by.dedup();
        flow.transformed_to.sort_by(|a,b|(&a.target_type,&a.via).cmp(&(&b.target_type,&b.via)));
        flow.transformed_to.dedup_by(|a,b|a.target_type==b.target_type&&a.via==b.via);
    }
    records.into_values().collect()
}
