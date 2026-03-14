// SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial

//! Layer violation detector — convention-based architectural layer inference.
//!
//! Detects cross-module references that violate architectural layering conventions.
//! Layers are inferred from directory names using common conventions:
//!
//! - **API layer:** `api/`, `handler/`, `route/`, `routes/`, `view/`, `views/`, `endpoint/`, `endpoints/`
//! - **Service layer:** `service/`, `services/`, `business/`, `logic/`, `usecase/`, `usecases/`
//! - **Data layer:** `db/`, `database/`, `repo/`, `repository/`, `model/`, `models/`, `dal/`
//!
//! The rule: API layer must NOT directly reference Data layer (must go through Service).
//! Service layer may reference Data layer. API layer may reference Service layer.
//!
//! **Confidence:** Always Moderate or below for convention-based inference.
//! HIGH confidence requires user-defined layer rules (not yet implemented).
//!
//! **Limitation:** Model/DTO type imports from data layer into API layer ARE flagged
//! at Moderate confidence. This is a known trade-off — convention-based detection
//! cannot distinguish data access calls from type imports. Users should review
//! flagged findings and suppress false positives.

use std::path::Path;

use crate::analyzer::diagnostic::*;
use crate::analyzer::patterns::exclusion::{is_excluded_symbol, is_test_path, relativize_path};
use crate::graph::Graph;
use crate::parser::ir::{EdgeKind, SymbolId};

/// Architectural layer inferred from directory naming conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Api,
    Service,
    Data,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::Api => write!(f, "API"),
            Layer::Service => write!(f, "Service"),
            Layer::Data => write!(f, "Data"),
        }
    }
}

/// Infer the architectural layer of a file from its directory path.
///
/// Returns `None` if the file is not in a recognized layer directory.
/// Only the first matching directory segment is used — nested layer
/// directories are not supported (e.g., `api/db/` uses `api/`).
fn infer_layer(file_path: &str) -> Option<Layer> {
    let normalized = file_path.replace('\\', "/");

    // Extract directory segments (everything except the filename)
    let dir_part = normalized.rsplit_once('/').map(|x| x.0).unwrap_or("");
    let segments: Vec<&str> = dir_part.split('/').collect();

    for segment in &segments {
        let seg = segment.to_lowercase();
        match seg.as_str() {
            "api" | "handler" | "handlers" | "route" | "routes" | "view" | "views" | "endpoint"
            | "endpoints" | "controller" | "controllers" => return Some(Layer::Api),
            "service" | "services" | "business" | "logic" | "usecase" | "usecases" => {
                return Some(Layer::Service)
            }
            "db" | "database" | "repo" | "repository" | "model" | "models" | "dal" => {
                return Some(Layer::Data)
            }
            _ => continue,
        }
    }

    None
}

/// Check if a reference from one layer to another is a violation.
///
/// The rule: API cannot directly reference Data (must go through Service).
/// All other combinations are allowed.
fn is_violation(from_layer: Layer, to_layer: Layer) -> bool {
    matches!((from_layer, to_layer), (Layer::Api, Layer::Data))
}

/// Detect layer violations in the analysis graph.
///
/// Scans all edges where both source and target symbols have inferred layers.
/// Flags references that cross forbidden layer boundaries (API → Data).
///
/// Confidence is always Moderate for convention-based inference.
/// Single-file projects and projects without recognizable layer directories
/// produce zero diagnostics.
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for (from_id, from_sym) in graph.all_symbols() {
        // Skip excluded symbols (test files, imports, dunders, etc.)
        if is_excluded_symbol(from_sym) {
            continue;
        }

        let from_path = from_sym.location.file.to_string_lossy();
        if is_test_path(&from_path) {
            continue;
        }

        let from_layer = match infer_layer(&from_path) {
            Some(l) => l,
            None => continue,
        };

        // Check all outgoing edges
        for edge in graph.edges_from(from_id) {
            // Only check Calls and References edges (not Contains/Transforms/etc.)
            if edge.kind != EdgeKind::Calls && edge.kind != EdgeKind::References {
                continue;
            }

            let to_id = edge.target;
            if to_id == SymbolId::default() {
                continue;
            }

            let to_sym = match graph.get_symbol(to_id) {
                Some(s) => s,
                None => continue,
            };

            let to_path = to_sym.location.file.to_string_lossy();
            if is_test_path(&to_path) {
                continue;
            }

            let to_layer = match infer_layer(&to_path) {
                Some(l) => l,
                None => continue,
            };

            // Skip same-layer references
            if from_layer == to_layer {
                continue;
            }

            if is_violation(from_layer, to_layer) {
                // Deduplicate by (from_symbol, to_symbol) pair
                let key = (from_id, to_id);
                if !seen.insert(key) {
                    continue;
                }

                let from_loc = relativize_path(&from_sym.location.file, project_root);
                let to_loc = relativize_path(&to_sym.location.file, project_root);

                diagnostics.push(Diagnostic {
                    id: String::new(),
                    pattern: DiagnosticPattern::LayerViolation,
                    severity: Severity::Warning,
                    confidence: Confidence::Moderate,
                    entity: from_sym.qualified_name.clone(),
                    message: format!(
                        "{} layer symbol `{}` directly references {} layer symbol `{}` — \
                         should route through Service layer",
                        from_layer, from_sym.name, to_layer, to_sym.name
                    ),
                    evidence: vec![
                        Evidence {
                            observation: format!(
                                "Source in {} layer (inferred from directory: {})",
                                from_layer, from_loc
                            ),
                            location: Some(format!("{}:{}", from_loc, from_sym.location.line)),
                            context: None,
                        },
                        Evidence {
                            observation: format!(
                                "Target in {} layer (inferred from directory: {})",
                                to_layer, to_loc
                            ),
                            location: Some(format!("{}:{}", to_loc, to_sym.location.line)),
                            context: None,
                        },
                        Evidence {
                            observation: "Layer inference is convention-based (directory names), \
                                          not user-defined rules"
                                .to_string(),
                            location: None,
                            context: Some(
                                "Confidence limited to Moderate without explicit layer configuration"
                                    .to_string(),
                            ),
                        },
                    ],
                    suggestion: format!(
                        "Route {} layer access through a Service layer intermediary instead of \
                         directly referencing {} layer from {}",
                        to_layer, to_layer, from_layer
                    ),
                    location: format!("{}:{}", from_loc, from_sym.location.line),
                });
            }
        }
    }

    diagnostics
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::ir::*;
    use crate::test_utils::*;

    // -- Layer inference tests --

    #[test]
    fn test_infer_layer_api_dir() {
        assert_eq!(infer_layer("api/handler.py"), Some(Layer::Api));
    }

    #[test]
    fn test_infer_layer_routes_dir() {
        assert_eq!(infer_layer("routes/users.py"), Some(Layer::Api));
    }

    #[test]
    fn test_infer_layer_handler_dir() {
        assert_eq!(infer_layer("handler/request.py"), Some(Layer::Api));
    }

    #[test]
    fn test_infer_layer_service_dir() {
        assert_eq!(infer_layer("service/user_service.py"), Some(Layer::Service));
    }

    #[test]
    fn test_infer_layer_business_dir() {
        assert_eq!(infer_layer("business/rules.py"), Some(Layer::Service));
    }

    #[test]
    fn test_infer_layer_db_dir() {
        assert_eq!(infer_layer("db/models.py"), Some(Layer::Data));
    }

    #[test]
    fn test_infer_layer_database_dir() {
        assert_eq!(infer_layer("database/queries.py"), Some(Layer::Data));
    }

    #[test]
    fn test_infer_layer_repo_dir() {
        assert_eq!(infer_layer("repo/user_repo.py"), Some(Layer::Data));
    }

    #[test]
    fn test_infer_layer_models_dir() {
        assert_eq!(infer_layer("models/user.py"), Some(Layer::Data));
    }

    #[test]
    fn test_infer_layer_unknown_dir() {
        assert_eq!(infer_layer("src/utils.py"), None);
    }

    #[test]
    fn test_infer_layer_no_dir() {
        assert_eq!(infer_layer("app.py"), None);
    }

    #[test]
    fn test_infer_layer_nested_api() {
        assert_eq!(infer_layer("src/api/v2/handler.py"), Some(Layer::Api));
    }

    #[test]
    fn test_infer_layer_windows_paths() {
        assert_eq!(infer_layer("src\\api\\handler.py"), Some(Layer::Api));
    }

    // -- Violation rule tests --

    #[test]
    fn test_api_to_data_is_violation() {
        assert!(is_violation(Layer::Api, Layer::Data));
    }

    #[test]
    fn test_api_to_service_not_violation() {
        assert!(!is_violation(Layer::Api, Layer::Service));
    }

    #[test]
    fn test_service_to_data_not_violation() {
        assert!(!is_violation(Layer::Service, Layer::Data));
    }

    #[test]
    fn test_data_to_api_not_violation() {
        // Reverse direction is not flagged — only top-down skip is a violation
        assert!(!is_violation(Layer::Data, Layer::Api));
    }

    // -- Detection integration tests --

    #[test]
    fn test_layer_violation_api_imports_db_directly() {
        let mut g = Graph::new();

        let handler = g.add_symbol(make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            "api/handler.py",
            1,
        ));

        let _service = g.add_symbol(make_symbol(
            "get_user",
            SymbolKind::Function,
            Visibility::Public,
            "service/user_service.py",
            1,
        ));

        let query_db = g.add_symbol(make_symbol(
            "query_db",
            SymbolKind::Function,
            Visibility::Public,
            "db/models.py",
            1,
        ));

        // Violation: api → db directly (skipping service)
        add_ref(
            &mut g,
            handler,
            query_db,
            ReferenceKind::Call,
            "api/handler.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::LayerViolation),
            "API handler importing directly from db layer MUST be flagged"
        );

        let violation = diagnostics
            .iter()
            .find(|d| d.pattern == DiagnosticPattern::LayerViolation)
            .unwrap();
        assert_eq!(
            violation.confidence,
            Confidence::Moderate,
            "Convention-based detection must be Moderate confidence, never High"
        );
    }

    #[test]
    fn test_layer_violation_proper_layering_no_diagnostic() {
        let mut g = Graph::new();

        let handler = g.add_symbol(make_symbol(
            "handle_request",
            SymbolKind::Function,
            Visibility::Public,
            "api/handler.py",
            1,
        ));
        let service = g.add_symbol(make_symbol(
            "get_user",
            SymbolKind::Function,
            Visibility::Public,
            "service/user_service.py",
            1,
        ));
        let db_fn = g.add_symbol(make_symbol(
            "query_db",
            SymbolKind::Function,
            Visibility::Public,
            "db/models.py",
            1,
        ));

        // Correct edges: api → service → db
        add_ref(
            &mut g,
            handler,
            service,
            ReferenceKind::Call,
            "api/handler.py",
        );
        add_ref(
            &mut g,
            service,
            db_fn,
            ReferenceKind::Call,
            "service/user_service.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        let violations: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
            .collect();
        assert!(
            violations.is_empty(),
            "Properly layered code must produce zero layer violations, got: {:?}",
            violations.iter().map(|d| &d.entity).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_layer_violation_flat_project_no_false_positives() {
        let mut g = Graph::new();

        let a = g.add_symbol(make_symbol(
            "process",
            SymbolKind::Function,
            Visibility::Public,
            "src/main.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "helper",
            SymbolKind::Function,
            Visibility::Public,
            "src/utils.py",
            1,
        ));
        add_ref(&mut g, a, b, ReferenceKind::Call, "src/main.py");

        let diagnostics = detect(&g, Path::new(""));
        let violations: Vec<_> = diagnostics
            .iter()
            .filter(|d| d.pattern == DiagnosticPattern::LayerViolation)
            .collect();
        assert!(
            violations.is_empty(),
            "Flat project with no layer directories must produce zero violations"
        );
    }

    #[test]
    fn test_layer_violation_model_import_in_api() {
        // Common pattern: API imports a data MODEL type (not a query function)
        // Convention-based detection flags this at Moderate confidence.
        // This is a documented trade-off — cannot distinguish type imports from
        // data access calls without deeper analysis.
        let mut g = Graph::new();

        let handler = g.add_symbol(make_symbol(
            "create_user",
            SymbolKind::Function,
            Visibility::Public,
            "api/handler.py",
            1,
        ));
        let user_model = g.add_symbol(make_symbol(
            "User",
            SymbolKind::Class,
            Visibility::Public,
            "db/models.py",
            1,
        ));
        // API references a CLASS in db layer
        add_ref(
            &mut g,
            handler,
            user_model,
            ReferenceKind::Read,
            "api/handler.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        // Model imports ARE flagged at Moderate confidence — this is the documented
        // trade-off for convention-based detection.
        assert!(
            diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::LayerViolation),
            "Model import from db layer into API is flagged at Moderate confidence"
        );
    }

    #[test]
    fn test_layer_violation_confidence_calibration() {
        let mut g = Graph::new();

        let handler = g.add_symbol(make_symbol(
            "handle",
            SymbolKind::Function,
            Visibility::Public,
            "api/routes.py",
            1,
        ));
        let db_call = g.add_symbol(make_symbol(
            "raw_sql",
            SymbolKind::Function,
            Visibility::Public,
            "database/queries.py",
            1,
        ));
        add_ref(
            &mut g,
            handler,
            db_call,
            ReferenceKind::Call,
            "api/routes.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        for d in &diagnostics {
            if d.pattern == DiagnosticPattern::LayerViolation {
                assert!(
                    d.confidence <= Confidence::Moderate,
                    "Convention-based layer violation must be Moderate or Low, \
                     never High. Got {:?} for entity '{}'",
                    d.confidence,
                    d.entity
                );
            }
        }
    }

    #[test]
    fn test_layer_violation_single_file_no_layers() {
        let mut g = Graph::new();
        let a = g.add_symbol(make_symbol(
            "fn_a",
            SymbolKind::Function,
            Visibility::Public,
            "app.py",
            1,
        ));
        let b = g.add_symbol(make_symbol(
            "fn_b",
            SymbolKind::Function,
            Visibility::Public,
            "app.py",
            10,
        ));
        add_ref(&mut g, a, b, ReferenceKind::Call, "app.py");

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .all(|d| d.pattern != DiagnosticPattern::LayerViolation),
            "Single-file project cannot have layer violations"
        );
    }

    #[test]
    fn test_layer_violation_alternative_dir_names() {
        // Tests breadth of directory-name conventions
        let mut g = Graph::new();

        let route_fn = g.add_symbol(make_symbol(
            "handle_route",
            SymbolKind::Function,
            Visibility::Public,
            "routes/users.py",
            1,
        ));
        let repo_fn = g.add_symbol(make_symbol(
            "find_user",
            SymbolKind::Function,
            Visibility::Public,
            "repo/user_repo.py",
            1,
        ));
        add_ref(
            &mut g,
            route_fn,
            repo_fn,
            ReferenceKind::Call,
            "routes/users.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics
                .iter()
                .any(|d| d.pattern == DiagnosticPattern::LayerViolation),
            "routes/ (API) → repo/ (Data) must be detected as violation"
        );
    }

    #[test]
    fn test_layer_violation_test_files_excluded() {
        let mut g = Graph::new();

        let test_fn = g.add_symbol(make_symbol(
            "test_handler",
            SymbolKind::Function,
            Visibility::Public,
            "api/test_handler.py",
            1,
        ));
        let db_fn = g.add_symbol(make_symbol(
            "query",
            SymbolKind::Function,
            Visibility::Public,
            "db/query.py",
            1,
        ));
        add_ref(
            &mut g,
            test_fn,
            db_fn,
            ReferenceKind::Call,
            "api/test_handler.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        assert!(
            diagnostics.is_empty(),
            "Test files should be excluded from layer violation detection"
        );
    }

    #[test]
    fn test_layer_violation_evidence_structure() {
        let mut g = Graph::new();

        let handler = g.add_symbol(make_symbol(
            "handle",
            SymbolKind::Function,
            Visibility::Public,
            "api/handler.py",
            5,
        ));
        let db_fn = g.add_symbol(make_symbol(
            "fetch",
            SymbolKind::Function,
            Visibility::Public,
            "db/repo.py",
            10,
        ));
        add_ref(
            &mut g,
            handler,
            db_fn,
            ReferenceKind::Call,
            "api/handler.py",
        );

        let diagnostics = detect(&g, Path::new(""));
        let d = diagnostics
            .iter()
            .find(|d| d.pattern == DiagnosticPattern::LayerViolation)
            .expect("Should have a layer violation");

        assert_eq!(d.evidence.len(), 3, "Must have 3 evidence entries");
        assert!(
            d.evidence[0].observation.contains("API"),
            "First evidence must mention source layer"
        );
        assert!(
            d.evidence[1].observation.contains("Data"),
            "Second evidence must mention target layer"
        );
        assert!(
            d.message.contains("Service"),
            "Message must mention routing through Service"
        );
        assert_eq!(d.severity, Severity::Warning);
    }

    #[test]
    fn test_layer_violation_controllers_dir() {
        assert_eq!(
            infer_layer("controllers/auth.py"),
            Some(Layer::Api),
            "controllers/ should map to API layer"
        );
    }

    #[test]
    fn test_layer_violation_dal_dir() {
        assert_eq!(
            infer_layer("dal/access.py"),
            Some(Layer::Data),
            "dal/ should map to Data layer"
        );
    }
}
