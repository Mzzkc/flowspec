// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Language-agnostic intermediate representation (IR) types.
//!
//! These types form the contract between language adapters (which produce IR
//! from tree-sitter ASTs) and the analysis engine (which never sees raw AST).
//! All four node types use generational IDs from slotmap for O(1) lookup and
//! safe incremental deletion.

use std::path::PathBuf;

use bincode::{Decode, Encode};
use serde::{Deserialize, Serialize};
use slotmap::new_key_type;

// ---------------------------------------------------------------------------
// ID types — generational keys for flat-table (ECS-inspired) storage
// ---------------------------------------------------------------------------

new_key_type! {
    /// Unique identifier for a [`Symbol`] in the analysis graph.
    pub struct SymbolId;
}

new_key_type! {
    /// Unique identifier for a [`Scope`] in the analysis graph.
    pub struct ScopeId;
}

new_key_type! {
    /// Unique identifier for a [`Boundary`] in the analysis graph.
    pub struct BoundaryId;
}

new_key_type! {
    /// Unique identifier for a [`Reference`] in the analysis graph.
    pub struct ReferenceId;
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

/// The kind of symbol extracted from source code.
///
/// Maps to the categories in `architecture.yaml:161`. Exactly 11 variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum SymbolKind {
    /// A standalone function (`def foo()`, `function foo()`, `fn foo()`).
    Function,
    /// A method bound to a class or impl block (`def method(self)`, `fn method(&self)`).
    Method,
    /// A class definition (`class Foo`, `class Foo {}`).
    Class,
    /// A struct definition (Rust `struct Foo {}`).
    Struct,
    /// A trait definition (Rust `trait Foo {}`).
    Trait,
    /// An interface or abstract class acting as a contract.
    Interface,
    /// A module or namespace (`mod foo`, Python file-level module).
    Module,
    /// A variable binding (`let x`, `x = 1`, `const x`).
    Variable,
    /// An immutable constant (`const X: u32 = 1`, `UPPER_CASE = ...`).
    Constant,
    /// A macro definition (Rust `macro_rules!`, decorators in other languages).
    Macro,
    /// An enum definition (Rust `enum Foo {}`).
    Enum,
}

/// Visibility of a symbol within its scope hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum Visibility {
    Public,
    Private,
    Crate,
    Protected,
}

/// The kind of reference edge between symbols.
///
/// Maps to `architecture.yaml:162`. Exactly 7 variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ReferenceKind {
    /// A function or method call (`func()`, `self.method()`, `Class()`).
    /// Produced by PythonAdapter's call-site detection. Maps to `EdgeKind::Calls`.
    Call,
    /// A variable read. Maps to `EdgeKind::References`.
    Read,
    /// A variable write / assignment. Maps to `EdgeKind::References`.
    Write,
    /// An import statement (`import x`, `from x import y`). Maps to `EdgeKind::References`.
    Import,
    /// An export / re-export. Maps to `EdgeKind::References`.
    Export,
    /// A trait/interface implementation. Maps to `EdgeKind::References`.
    Implement,
    /// A derive macro or attribute. Maps to `EdgeKind::References`.
    Derive,
}

/// The kind of boundary crossed by a reference.
///
/// Exactly 5 variants per `architecture.yaml:163`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum BoundaryKind {
    Module,
    Package,
    Network,
    Serialization,
    Ffi,
}

/// The kind of lexical scope.
///
/// Exactly 4 variants per `architecture.yaml:164`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ScopeKind {
    File,
    Module,
    Function,
    Block,
}

/// The kind of edge in the analysis graph.
///
/// Exactly 6 variants per `architecture.yaml:165-171`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum EdgeKind {
    Calls,
    References,
    Contains,
    Crosses,
    Transforms,
    FlowsTo,
}

/// How completely a symbol or reference has been resolved.
///
/// The analysis engine reports resolution status honestly: false negatives
/// are worse than false positives. When in doubt, report as `Partial`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub enum ResolutionStatus {
    /// Fully resolved — the target is known with certainty.
    Resolved,
    /// Partially resolved — some information is known but incomplete.
    /// The string describes why resolution is partial.
    Partial(String),
    /// Unresolved — the target could not be determined.
    Unresolved,
}

// ---------------------------------------------------------------------------
// Location
// ---------------------------------------------------------------------------

/// Source location of an IR node. Uses 1-based line/column numbers
/// (matching editor conventions and `file:line` format in manifests).
///
/// Tree-sitter provides 0-based positions; language adapters convert to 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode)]
pub struct Location {
    /// Path to the source file, relative to the project root.
    pub file: PathBuf,
    /// Start line (1-based).
    pub line: u32,
    /// Start column (1-based).
    pub column: u32,
    /// End line (1-based).
    pub end_line: u32,
    /// End column (1-based).
    pub end_column: u32,
}

// ---------------------------------------------------------------------------
// Core node types
// ---------------------------------------------------------------------------

/// A named entity in the source code: function, class, variable, etc.
///
/// Symbols are the primary nodes in the analysis graph. They are stored in
/// flat tables (slotmap arenas) for O(1) lookup and cache-friendly iteration.
/// The `id` field is assigned by the graph upon insertion; adapters leave it
/// as a default key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Symbol {
    /// Unique identifier assigned by the graph.
    pub id: SymbolId,
    /// What kind of source entity this is.
    pub kind: SymbolKind,
    /// Simple name (e.g., `process_data`).
    pub name: String,
    /// Fully qualified name (e.g., `pipeline::DataProcessor::process_data`).
    pub qualified_name: String,
    /// Visibility within the scope hierarchy.
    pub visibility: Visibility,
    /// Optional type signature (e.g., `(data: list) -> dict`).
    pub signature: Option<String>,
    /// Where this symbol is defined in source code.
    pub location: Location,
    /// How completely this symbol has been resolved.
    pub resolution: ResolutionStatus,
    /// The scope that contains this symbol.
    pub scope: ScopeId,
    /// Decorators, attributes, or annotations on this symbol.
    pub annotations: Vec<String>,
}

/// A directed edge between two symbols: call, import, read, etc.
///
/// References carry resolution status because the target of an import or
/// call may not be fully resolvable (e.g., dynamic dispatch, star imports).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reference {
    /// Unique identifier assigned by the graph.
    pub id: ReferenceId,
    /// The symbol where the reference originates.
    pub from: SymbolId,
    /// The symbol being referenced.
    pub to: SymbolId,
    /// What kind of reference this is.
    pub kind: ReferenceKind,
    /// Where the reference occurs in source code.
    pub location: Location,
    /// How completely the reference target has been resolved.
    pub resolution: ResolutionStatus,
}

/// A boundary between scopes that a reference may cross.
///
/// Boundary crossings are important for diagnostics like `layer_violation`
/// and for understanding the architecture of a codebase.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boundary {
    /// Unique identifier assigned by the graph.
    pub id: BoundaryId,
    /// What kind of boundary this is.
    pub kind: BoundaryKind,
    /// The scope on the source side of the boundary.
    pub from_scope: ScopeId,
    /// The scope on the target side of the boundary.
    pub to_scope: ScopeId,
    /// Where the boundary crossing occurs.
    pub location: Location,
}

/// A lexical scope in the source code: file, module, function, or block.
///
/// Scopes form a tree via `parent` pointers. The root scope for each file
/// has `parent: None`. Symbols belong to exactly one scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    /// Unique identifier assigned by the graph.
    pub id: ScopeId,
    /// What kind of scope this is.
    pub kind: ScopeKind,
    /// Parent scope (None for file-level scope).
    pub parent: Option<ScopeId>,
    /// Name of this scope (filename for File, function name for Function, etc.).
    pub name: String,
    /// Where this scope spans in source code.
    pub location: Location,
}

// ---------------------------------------------------------------------------
// Edge (for adjacency lists)
// ---------------------------------------------------------------------------

/// An edge in the graph's adjacency list, pointing to a target symbol.
///
/// Edges are stored in bidirectional adjacency maps: `outgoing[from]` and
/// `incoming[to]`. The optional `reference_id` links back to the full
/// [`Reference`] record for detailed information.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    /// What kind of relationship this edge represents.
    pub kind: EdgeKind,
    /// The target symbol.
    pub target: SymbolId,
    /// Back-pointer to the full Reference record, if this edge was created
    /// from a Reference.
    pub reference_id: Option<ReferenceId>,
}

// ---------------------------------------------------------------------------
// Parse result (adapter output)
// ---------------------------------------------------------------------------

/// The output of a language adapter's `parse_file` method.
///
/// Contains all IR nodes extracted from a single source file. The graph
/// engine inserts these into its flat tables, assigning real IDs.
#[derive(Debug, Clone, Default)]
pub struct ParseResult {
    /// Symbols extracted from the file.
    pub symbols: Vec<Symbol>,
    /// References (calls, imports, etc.) found in the file.
    pub references: Vec<Reference>,
    /// Scopes (file, function, class, block) found in the file.
    pub scopes: Vec<Scope>,
    /// Boundary crossings found in the file.
    pub boundaries: Vec<Boundary>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // 1.1 SymbolKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_symbol_kind_all_variants_constructible() {
        let variants = vec![
            SymbolKind::Function,
            SymbolKind::Method,
            SymbolKind::Class,
            SymbolKind::Struct,
            SymbolKind::Trait,
            SymbolKind::Interface,
            SymbolKind::Module,
            SymbolKind::Variable,
            SymbolKind::Constant,
            SymbolKind::Macro,
            SymbolKind::Enum,
        ];
        assert_eq!(
            variants.len(),
            11,
            "SymbolKind must have exactly 11 variants per spec"
        );
    }

    #[test]
    fn test_symbol_kind_debug_representation() {
        let kind = SymbolKind::Function;
        let debug = format!("{:?}", kind);
        assert!(
            debug.contains("Function"),
            "Debug repr must be human-readable, got: {}",
            debug
        );
    }

    #[test]
    fn test_symbol_kind_equality() {
        assert_eq!(SymbolKind::Function, SymbolKind::Function);
        assert_ne!(SymbolKind::Function, SymbolKind::Method);
    }

    #[test]
    fn test_symbol_kind_clone() {
        let kind = SymbolKind::Class;
        let cloned = kind.clone();
        assert_eq!(kind, cloned);
    }

    // -----------------------------------------------------------------------
    // 1.2 Visibility
    // -----------------------------------------------------------------------

    #[test]
    fn test_visibility_all_variants() {
        let variants = vec![
            Visibility::Public,
            Visibility::Private,
            Visibility::Crate,
            Visibility::Protected,
        ];
        assert_eq!(variants.len(), 4);
    }

    // -----------------------------------------------------------------------
    // 1.3 ReferenceKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_reference_kind_all_variants() {
        let variants = vec![
            ReferenceKind::Call,
            ReferenceKind::Read,
            ReferenceKind::Write,
            ReferenceKind::Import,
            ReferenceKind::Export,
            ReferenceKind::Implement,
            ReferenceKind::Derive,
        ];
        assert_eq!(
            variants.len(),
            7,
            "ReferenceKind must have exactly 7 variants per spec"
        );
    }

    // -----------------------------------------------------------------------
    // 1.4 BoundaryKind and ScopeKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_boundary_kind_all_variants() {
        let variants = vec![
            BoundaryKind::Module,
            BoundaryKind::Package,
            BoundaryKind::Network,
            BoundaryKind::Serialization,
            BoundaryKind::Ffi,
        ];
        assert_eq!(variants.len(), 5);
    }

    #[test]
    fn test_scope_kind_all_variants() {
        let variants = vec![
            ScopeKind::File,
            ScopeKind::Module,
            ScopeKind::Function,
            ScopeKind::Block,
        ];
        assert_eq!(variants.len(), 4);
    }

    // -----------------------------------------------------------------------
    // 1.5 EdgeKind
    // -----------------------------------------------------------------------

    #[test]
    fn test_edge_kind_all_variants() {
        let variants = vec![
            EdgeKind::Calls,
            EdgeKind::References,
            EdgeKind::Contains,
            EdgeKind::Crosses,
            EdgeKind::Transforms,
            EdgeKind::FlowsTo,
        ];
        assert_eq!(
            variants.len(),
            6,
            "EdgeKind must have exactly 6 variants per spec"
        );
    }

    // -----------------------------------------------------------------------
    // 1.6 ResolutionStatus
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolution_status_resolved() {
        let status = ResolutionStatus::Resolved;
        assert_eq!(status, ResolutionStatus::Resolved);
    }

    #[test]
    fn test_resolution_status_partial_with_reason() {
        let status = ResolutionStatus::Partial("dynamic attribute".to_string());
        match &status {
            ResolutionStatus::Partial(reason) => assert_eq!(reason, "dynamic attribute"),
            _ => panic!("Expected Partial variant"),
        }
    }

    #[test]
    fn test_resolution_status_unresolved() {
        let status = ResolutionStatus::Unresolved;
        assert_eq!(status, ResolutionStatus::Unresolved);
    }

    #[test]
    fn test_resolution_status_partial_empty_reason() {
        let status = ResolutionStatus::Partial(String::new());
        match &status {
            ResolutionStatus::Partial(reason) => assert!(reason.is_empty()),
            _ => panic!("Expected Partial variant"),
        }
    }

    #[test]
    fn test_resolution_status_partial_long_reason() {
        let long_reason = "x".repeat(10_000);
        let status = ResolutionStatus::Partial(long_reason.clone());
        match &status {
            ResolutionStatus::Partial(reason) => assert_eq!(reason.len(), 10_000),
            _ => panic!("Expected Partial variant"),
        }
    }

    #[test]
    fn test_resolution_status_equality() {
        let a = ResolutionStatus::Partial("reason A".to_string());
        let b = ResolutionStatus::Partial("reason B".to_string());
        assert_ne!(a, b);

        let c = ResolutionStatus::Partial("same".to_string());
        let d = ResolutionStatus::Partial("same".to_string());
        assert_eq!(c, d);
    }

    // -----------------------------------------------------------------------
    // 1.7 Location
    // -----------------------------------------------------------------------

    #[test]
    fn test_location_construction() {
        let loc = Location {
            file: PathBuf::from("src/main.py"),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 20,
        };
        assert_eq!(loc.line, 1);
        assert_eq!(loc.file, PathBuf::from("src/main.py"));
    }

    #[test]
    fn test_location_single_character_span() {
        let loc = Location {
            file: PathBuf::from("a.py"),
            line: 5,
            column: 10,
            end_line: 5,
            end_column: 11,
        };
        assert_eq!(loc.end_column - loc.column, 1);
    }

    #[test]
    fn test_location_multiline_span() {
        let loc = Location {
            file: PathBuf::from("big.py"),
            line: 1,
            column: 1,
            end_line: 100,
            end_column: 1,
        };
        assert!(loc.end_line > loc.line);
    }

    #[test]
    fn test_location_line_zero() {
        // Location is a plain struct; validation is the adapter's responsibility.
        // This test documents that zero is representable.
        let loc = Location {
            file: PathBuf::from("edge.py"),
            line: 0,
            column: 0,
            end_line: 0,
            end_column: 0,
        };
        assert_eq!(loc.line, 0);
    }

    #[test]
    fn test_location_end_before_start() {
        // Adversarial: end_line < line — representable but should be caught upstream.
        let loc = Location {
            file: PathBuf::from("corrupt.py"),
            line: 10,
            column: 5,
            end_line: 5,
            end_column: 1,
        };
        assert!(
            loc.end_line < loc.line,
            "Constructed Location with end before start — is this intended?"
        );
    }

    #[test]
    fn test_location_unicode_file_path() {
        let loc = Location {
            file: PathBuf::from("src/données/café.py"),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 10,
        };
        assert!(loc.file.to_str().unwrap().contains("café"));
    }

    #[test]
    fn test_location_deeply_nested_path() {
        let deep = "a/".repeat(200) + "file.py";
        let loc = Location {
            file: PathBuf::from(&deep),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 1,
        };
        assert!(loc.file.components().count() > 200);
    }

    // -----------------------------------------------------------------------
    // 1.11 Serialization round-trip (bincode)
    // -----------------------------------------------------------------------

    #[test]
    fn test_symbol_kind_serialize_roundtrip() {
        let kind = SymbolKind::Function;
        let bytes = bincode::encode_to_vec(&kind, bincode::config::standard()).unwrap();
        let (decoded, _): (SymbolKind, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(kind, decoded);
    }

    #[test]
    fn test_resolution_status_serialize_all_variants() {
        for status in [
            ResolutionStatus::Resolved,
            ResolutionStatus::Partial("test reason".to_string()),
            ResolutionStatus::Unresolved,
        ] {
            let bytes = bincode::encode_to_vec(&status, bincode::config::standard()).unwrap();
            let (decoded, _): (ResolutionStatus, _) =
                bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
            assert_eq!(status, decoded);
        }
    }

    #[test]
    fn test_location_serialize_with_unicode_path() {
        let loc = Location {
            file: PathBuf::from("données/café.py"),
            line: 1,
            column: 1,
            end_line: 1,
            end_column: 10,
        };
        let bytes = bincode::encode_to_vec(&loc, bincode::config::standard()).unwrap();
        let (decoded, _): (Location, _) =
            bincode::decode_from_slice(&bytes, bincode::config::standard()).unwrap();
        assert_eq!(loc.file, decoded.file);
    }
}
