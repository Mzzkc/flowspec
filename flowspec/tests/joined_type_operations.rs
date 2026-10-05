use flowspec::{analyze,Config};
fn report(extension:&str,source:&str)->flowspec::AnalysisReport {
 let root=tempfile::tempdir().unwrap();std::fs::write(root.path().join(format!("model.{extension}")),source).unwrap();analyze(root.path(),&Config::default(),&[]).unwrap().manifest
}
#[test]
fn javascript_new_is_a_bound_creation() {
 let r=report("js","class Raw {}\nfunction make() { return new Raw(); }\n");
 assert!(r.type_flows.iter().any(|f| f.type_name=="model.js::Raw" && f.created_at==["model.js::make"]));
}
#[test]
fn native_typescript_typed_runtime_consumer_is_not_an_annotation_only_flow() {
 let r=report("ts","class Raw {}\nfunction make(): Raw { return new Raw(); }\nfunction consume(value: Raw) { return value; }\n");
 assert!(r.type_flows.iter().any(|f|f.type_name=="model.ts::Raw" && f.created_at==["model.ts::make"] && f.consumed_by==["model.ts::consume"]));
 let rival=report("ts","class Raw {}\nfunction annotation_only(value: Raw) {}\nfunction shadowed(Raw: Function) { return new Raw(); }\n");
 assert!(rival.type_flows.iter().all(|f|f.created_at.is_empty() && f.consumed_by.is_empty()));
}
#[test]
fn rust_unmodelled_construction_has_located_type_operation_debt() {
 let r=report("rs","struct Raw {}\nfn make() -> Raw { Raw {} }\n");
 let a=r.authority.unwrap();
 assert!(a["losses"].as_array().unwrap().iter().any(|l|l["reason"]=="type_operation_unresolved" && !l["ranges"].as_array().unwrap().is_empty()));
 assert_eq!(a["execution"],"incomplete");
}
