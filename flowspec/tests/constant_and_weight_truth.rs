use flowspec::{analyze,Config};
use flowspec::parser::ir::EdgeKind;
#[test]
fn returned_constant_read_binds_exact_authority_but_parameter_shadow_does_not() {
    for (parameter,expected) in [("x: u64",true),("EXP_MASK: u64",false)] {
        let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join("source.rs"),format!("const EXP_MASK: u64 = 42; pub fn api({parameter}) -> u64 {{ return EXP_MASK; }}")).unwrap();
        let r=analyze(d.path(),&Config::default(),&[]).unwrap();let (constant,_)=r.graph.all_symbols().find(|(_,s)|s.name=="EXP_MASK").unwrap();let (api,_)=r.graph.all_symbols().find(|(_,s)|s.name=="api").unwrap();
        let edge=r.graph.edges_from(api).iter().any(|e|e.kind==EdgeKind::References && e.target==constant);assert_eq!(edge,expected,"parameter shadow is not a consumer of unrelated outer constant");
    }
}
#[test]
fn calls_and_returns_never_inflate_legacy_duplication_weight() {
    for (ext,src) in [
        ("py","def h1(x):\n    return x\ndef h2(x):\n    return x\ndef h3(x):\n    return x\ndef a(x):\n    h1(x)\n    h2(x)\n    return h3(x)\ndef b(x):\n    h1(x)\n    h2(x)\n    return h3(x)\n"),
        ("js","function h1(x){return x;} function h2(x){return x;} function h3(x){return x;} function a(x){h1(x);h2(x);return h3(x);} function b(x){h1(x);h2(x);return h3(x);}"),
        ("ts","function h1(x:boolean):boolean{return x;} function h2(x:boolean):boolean{return x;} function h3(x:boolean):boolean{return x;} function a(x:boolean):boolean{h1(x);h2(x);return h3(x);} function b(x:boolean):boolean{h1(x);h2(x);return h3(x);}"),
        ("rs","fn h1(x: bool)->bool{x} fn h2(x: bool)->bool{x} fn h3(x: bool)->bool{x} fn a(x: bool)->bool{h1(x);h2(x);return h3(x);} fn b(x: bool)->bool{h1(x);h2(x);return h3(x);}")
    ] {
        let d=tempfile::tempdir().unwrap();std::fs::write(d.path().join(format!("source.{ext}")),src).unwrap();let r=analyze(d.path(),&Config::default(),&[]).unwrap();assert!(r.manifest.diagnostics.iter().all(|d|d.pattern!="duplication"),"shared three-callee wrappers have W0 in {ext}: {:?}",r.manifest.diagnostics);
    }
}
