use flowspec::{analyze,Config};
fn analyze_source(source:&str)->flowspec::AnalysisResult {let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("source.rs"),source).unwrap();analyze(d.path(),&Config::default(),&[]).unwrap()}
#[test]
fn public_method_syntax_on_unescaped_private_owner_is_dead_and_unregistered_impl() {
    let r=analyze_source("struct Token; impl Token { pub fn abandoned(&self) -> bool { true } }");
    let a=r.manifest.authority.unwrap();
    for pattern in ["data_dead_end","orphaned_impl"] {
        let f:Vec<_>=a["findings"]["all"].as_array().unwrap().iter().filter(|f|f["pattern_id"]==pattern && f["subject"]["qualified_name"].as_str().is_some_and(|n|n.ends_with("Token::abandoned"))).collect();
        assert_eq!(f.len(),1,"actual private implementation relation: {a}");
        assert_eq!(f[0]["evidence"][0]["owner_escape"],"none");
    }
}
#[test]
fn exported_constructed_and_registered_owners_defeat_private_absence_proof() {
    for source in [
        "pub struct Token; impl Token { pub fn abandoned(&self) -> bool { true } }",
        "struct Token; impl Token { pub fn abandoned(&self) -> bool { true } } pub fn api() -> bool { Token.abandoned() }",
        "struct Token; impl Token { pub fn abandoned(&self) -> bool { true } } pub fn register() { let dispatch = Token::abandoned; }",
    ] {
        let r=analyze_source(source);
        assert!(r.manifest.diagnostics.iter().all(|d| !(d.pattern=="orphaned_impl" && d.entity.ends_with("Token::abandoned"))),"unsupported/exported/consumed implementation must not become guessed orphan: {:?}",r.manifest.diagnostics);
        let a=r.manifest.authority.unwrap();
        assert!(a["findings"]["all"].as_array().unwrap().iter().all(|f|!f["subject"]["qualified_name"].as_str().is_some_and(|n|n.ends_with("Token::abandoned"))));
    }
}
#[test]
fn unrelated_owner_same_method_name_preserves_the_dead_private_implementation() {
    let r=analyze_source("struct Token; impl Token { pub fn check(&self) -> bool { true } } pub struct Other; impl Other { pub fn check(&self) -> bool { true } } pub fn api(x: &Other) -> bool { x.check() }");
    let a=r.manifest.authority.unwrap();
    for p in ["data_dead_end","orphaned_impl"] {
        let f:Vec<_>=a["findings"]["all"].as_array().unwrap().iter().filter(|f|f["pattern_id"]==p && f["subject"]["qualified_name"].as_str().is_some_and(|n|n.ends_with("Token::check"))).collect();assert_eq!(f.len(),1,"other owner's consumer is unrelated: {a}");
    }
}
