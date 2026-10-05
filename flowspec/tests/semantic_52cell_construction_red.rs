//! Compiling causal RED for 52-cell semantic construction.
//!
//! Obligation:
//! 1. Rust used constants (e.g. EXP_MASK) must bind as actual references and not be data_dead_end.
//! 2. Methods on types with receiver callers (e.g. Graph::add_symbol, Completeness::reject)
//!    must not be flagged as orphaned_impl.
//! 3. Inherent public methods on public types are not implementation obligations.
//! 4. Pattern state exposes all 52 cells (13 canonical patterns x 4 languages).

use std::fs;
use flowspec::{analyze, Config};
use tempfile::TempDir;

#[test]
fn rust_used_constant_binds_as_reference_and_is_not_data_dead_end() {
    let dir = TempDir::new().unwrap();
    let src = r#"
pub const EXP_MASK: u64 = 0x7FF0_0000_0000_0000;

pub fn extract_exponent(val: u64) -> u64 {
    val & EXP_MASK
}
"#;
    fs::write(dir.path().join("lib.rs"), src).unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".to_string()]).unwrap();
    let dde: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "data_dead_end" && d.entity.contains("EXP_MASK"))
        .collect();
    assert!(
        dde.is_empty(),
        "EXP_MASK is used in extract_exponent, must not be diagnosed as data_dead_end: {dde:?}"
    );
}

#[test]
fn rust_public_methods_with_receiver_calls_are_not_orphaned_impl() {
    let dir = TempDir::new().unwrap();
    let src = r#"
pub struct Graph {
    pub count: usize,
}

impl Graph {
    pub fn add_symbol(&mut self, n: usize) -> usize {
        self.count += n;
        self.count
    }
}

pub fn populate(graph: &mut Graph) -> usize {
    graph.add_symbol(42)
}
"#;
    fs::write(dir.path().join("lib.rs"), src).unwrap();
    let result = analyze(dir.path(), &Config::default(), &["rust".to_string()]).unwrap();
    let orphans: Vec<_> = result
        .manifest
        .diagnostics
        .iter()
        .filter(|d| d.pattern == "orphaned_impl" && d.entity.contains("add_symbol"))
        .collect();
    assert!(
        orphans.is_empty(),
        "Graph::add_symbol is called via receiver graph.add_symbol, must not be orphaned_impl: {orphans:?}"
    );
}

#[test]
fn all_fifty_two_pattern_state_cells_are_present() {
    let dir = TempDir::new().unwrap();
    fs::write(dir.path().join("app.py"), "x = 1\n").unwrap();
    fs::write(dir.path().join("app.js"), "const x = 1;\n").unwrap();
    fs::write(dir.path().join("app.ts"), "const x: number = 1;\n").unwrap();
    fs::write(dir.path().join("lib.rs"), "pub const X: u32 = 1;\n").unwrap();
    let result = analyze(
        dir.path(),
        &Config::default(),
        &[
            "python".to_string(),
            "javascript".to_string(),
            "typescript".to_string(),
            "rust".to_string(),
        ],
    )
    .unwrap();
    let authority = result.manifest.authority.as_ref().expect("authority");
    let pattern_state = authority["pattern_state"].as_array().expect("pattern_state array");
    assert_eq!(pattern_state.len(), 13, "Must have exactly 13 canonical patterns");
    let mut total_cells = 0;
    for p in pattern_state {
        let cells = p["cells"].as_array().expect("cells array");
        total_cells += cells.len();
        let languages: Vec<_> = cells.iter().map(|c| c["language"].as_str().unwrap()).collect();
        assert!(languages.contains(&"python"));
        assert!(languages.contains(&"javascript"));
        assert!(languages.contains(&"typescript"));
        assert!(languages.contains(&"rust"));
    }
    assert_eq!(total_cells, 52, "Must have exactly 52 cells (13 patterns x 4 languages)");
}
