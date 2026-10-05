//! Additional source-first controls for transitive visibility and lexical use origins.
use flowspec::{analyze, Config};
use serde_json::Value;
fn report(source:&str, ext:&str)->Value{
 let r=tempfile::tempdir().unwrap();std::fs::write(r.path().join(format!("source.{ext}")),source).unwrap();analyze(r.path(),&Config::default(),&[]).unwrap().manifest.authority.unwrap()
}
#[test]
fn public_function_in_private_module_is_not_an_external_api() {
 let r=report("mod internal { pub fn abandoned() -> bool { true } }", "rs");
 assert!(r["findings"]["all"].as_array().unwrap().iter().any(|f|f["pattern_id"]=="data_dead_end" && f["subject"]["qualified_name"].as_str().is_some_and(|n|n.ends_with("internal::abandoned"))),"private-module visibility must propagate: {r}");
}
#[test]
fn consumed_and_reexported_private_module_functions_are_not_dead() {
 for source in ["mod internal { pub fn active() -> bool { true } } pub fn api() -> bool { internal::active() }", "mod internal { pub fn active() -> bool { true } } pub use internal::active;"] {
 let r=report(source,"rs");assert!(r["findings"]["all"].as_array().unwrap().iter().all(|f|f["subject"]["qualified_name"].as_str().is_none_or(|n|!n.ends_with("internal::active"))),"resolved visibility/consumer: {r}");
 }
}
#[test]
fn mutating_a_local_is_not_a_proven_unused_produced_value() {
 let r=report("export function f(input) { let v = input === null; v = false; return input; }","js");assert!(r["findings"]["all"].as_array().unwrap().iter().all(|f|f["subject"]["name"]!="v"));
}
#[test]
fn getter_and_coercive_production_do_not_claim_a_pure_boolean_proof() {
 for expr in ["input.getter === null","input == null","getValue() === null"] {
 let r=report(&format!("export function f(input) {{ const v = {expr}; return input; }}"),"js");
 assert!(r["findings"]["all"].as_array().unwrap().iter().all(|f|f["subject"]["name"]!="v"));
 assert!(r["losses"].as_array().unwrap().iter().any(|l|l["reason"]=="value_effect_unresolved"));
 }
}
