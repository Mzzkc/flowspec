//! Opt-in, debug-only analyzer fault probe. Operations are recorded at the
//! discovery, source-read, adapter, and resolver sites that actually run.
use crate::parser::ir::Location;
use crate::{AnalysisResult, Config, FlowspecError};
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// One fault at a single analyzer operation.
pub enum FaultCase {
    WalkerError,
    ReadPermissionError,
    DepthTruncation,
    ChangedDuringRead,
    UnsupportedRequiredConstruct,
    UnresolvedImport,
    UnresolvedCall,
    UnresolvedType,
    UnresolvedReference,
}

impl FromStr for FaultCase {
    type Err = String;
    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "walker-error" => Ok(Self::WalkerError),
            "read-permission-error" => Ok(Self::ReadPermissionError),
            "depth-truncation" => Ok(Self::DepthTruncation),
            "changed-during-read" => Ok(Self::ChangedDuringRead),
            "unsupported-required-construct" => Ok(Self::UnsupportedRequiredConstruct),
            "unresolved-import" => Ok(Self::UnresolvedImport),
            "unresolved-call" => Ok(Self::UnresolvedCall),
            "unresolved-type" => Ok(Self::UnresolvedType),
            "unresolved-reference" => Ok(Self::UnresolvedReference),
            _ => Err(format!("unknown fault case: {value}")),
        }
    }
}

#[derive(Debug, Clone)]
/// A single validated target; no report content or operation transcript is caller supplied.
pub struct FaultPlan {
    case: FaultCase,
    target: PathBuf,
    nonce: String,
}

impl FaultPlan {
    /// Validate a relative source target and nonempty gate nonce.
    pub fn new(case: FaultCase, target: &str, nonce: &str) -> Result<Self, String> {
        let path = Path::new(target);
        if path.as_os_str().is_empty()
            || path.is_absolute()
            || path
                .components()
                .any(|component| !matches!(component, std::path::Component::Normal(_)))
            || nonce.is_empty()
        {
            return Err(
                "target must be a nonempty relative source path and nonce must be nonempty".into(),
            );
        }
        Ok(Self {
            case,
            target: path.to_path_buf(),
            nonce: nonce.to_string(),
        })
    }
}

/// Actual result of the shared analyzer core plus its core-emitted operations.
pub struct ProbeResult {
    pub analysis: AnalysisResult,
    pub operations: Vec<Value>,
}

struct State {
    root: PathBuf,
    plan: FaultPlan,
    operations: Vec<Value>,
    losses: Vec<Value>,
    sources: HashMap<PathBuf, String>,
    fired: bool,
}
thread_local! { static STATE: RefCell<Option<State>> = const { RefCell::new(None) }; }

/// Run the same core as native analyze with one opt-in operation fault.
pub fn analyze_with_plan(
    root: &Path,
    config: &Config,
    languages: &[String],
    plan: FaultPlan,
) -> Result<ProbeResult, FlowspecError> {
    STATE.with(|slot| {
        assert!(slot.borrow().is_none(), "nested fault probe");
        *slot.borrow_mut() = Some(State {
            root: root.to_path_buf(),
            plan,
            operations: Vec::new(),
            losses: Vec::new(),
            sources: HashMap::new(),
            fired: false,
        });
    });
    let result = crate::analyze_core(root, config, languages);
    let state = STATE.with(|slot| slot.borrow_mut().take().expect("probe state"));
    result.map(|analysis| ProbeResult {
        analysis,
        operations: state.operations,
    })
}

fn at_target(state: &State, path: &Path, case: FaultCase) -> bool {
    state.plan.case == case
        && path.strip_prefix(&state.root).ok() == Some(state.plan.target.as_path())
        && !state.fired
}

fn operation(
    state: &mut State,
    name: &str,
    path: &Path,
    line: Option<u64>,
    callsite: &str,
    result: &str,
) {
    let index = state.operations.len();
    state.operations.push(json!({"index":index,"operation":name,"case":format!("{:?}",state.plan.case),
        "gate_nonce":state.plan.nonce,"target_path_key":crate::identity::path_key(&state.root,path),
        "line":line,"callsite":callsite,"result":result}));
}
fn loss(
    state: &mut State,
    reason: &str,
    path: &Path,
    range: Option<Value>,
    stage: &str,
    origin: Option<&str>,
) {
    // All probe ranges bind the bytes read by the real core, including AST
    // nodes spanning newlines. Caller-supplied coordinates are never evidence.
    let range = range.map(|range| {
        state.sources.get(path).map_or_else(
            || range.clone(),
            |content| {
                crate::report_authority::offset_range(
                    &crate::identity::path_key(&state.root, path),
                    content,
                    range["start_byte"].as_u64().unwrap_or(0),
                    range["end_byte"].as_u64().unwrap_or(0),
                )
            },
        )
    });
    let mut entry = crate::report_authority::loss(
        reason,
        crate::identity::path_key(&state.root, path),
        range,
        stage,
    );
    if let Some(origin) = origin {
        entry["origin"] = json!(origin);
    }
    state.losses.push(entry);
}

pub(crate) fn drain_losses() -> Vec<Value> {
    STATE.with(|slot| {
        slot.borrow_mut()
            .as_mut()
            .map(|s| std::mem::take(&mut s.losses))
            .unwrap_or_default()
    })
}

pub(crate) fn walker_candidate(path: &Path) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if !at_target(state, path, FaultCase::WalkerError) {
            return false;
        }
        operation(
            state,
            "discovery.candidate",
            path,
            None,
            "lib.discover_source_files.walker",
            "yielded",
        );
        operation(
            state,
            "discovery.inject_error",
            path,
            None,
            "lib.discover_source_files.walker",
            "error",
        );
        loss(state, "discovery_error", path, None, "discovery", None);
        state.fired = true;
        true
    })
}

pub(crate) fn read_before(path: &Path) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        let permission = at_target(state, path, FaultCase::ReadPermissionError);
        let changed = at_target(state, path, FaultCase::ChangedDuringRead);
        if permission {
            operation(
                state,
                "read.before",
                path,
                None,
                "lib.selected_source_read",
                "started",
            );
            operation(
                state,
                "read.inject_permission_denied",
                path,
                None,
                "lib.selected_source_read",
                "permission_denied",
            );
            loss(state, "read_error", path, None, "read", None);
            state.fired = true;
        } else if changed {
            operation(
                state,
                "read.before_metadata",
                path,
                None,
                "lib.selected_source_read",
                "real_metadata",
            );
        }
        permission
    })
}

pub(crate) fn read_after(path: &Path, content: &str) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        state
            .sources
            .insert(path.to_path_buf(), content.to_string());
        if !at_target(state, path, FaultCase::ChangedDuringRead) {
            return false;
        }
        operation(
            state,
            "read.real_bytes",
            path,
            None,
            "lib.selected_source_read",
            "consumed",
        );
        if let Some(last) = state.operations.last_mut() {
            last["raw_sha256"] = json!(crate::identity::digest(content.as_bytes()));
            last["bytes_consumed"] = json!(content.len());
        }
        operation(
            state,
            "read.inject_after_metadata_mismatch",
            path,
            None,
            "lib.selected_source_read",
            "changed",
        );
        loss(state, "changed_during_run", path, None, "read", None);
        state.fired = true;
        true
    })
}

pub(crate) fn js_node(path: &Path, kind: &str, line: u64, start: u64, end: u64) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if !at_target(state, path, FaultCase::DepthTruncation) || kind != "return_statement" {
            return false;
        }
        operation(
            state,
            "parse.visit_return_statement",
            path,
            Some(line),
            "parser.javascript.visit_node",
            "visited",
        );
        operation(
            state,
            "parse.inject_depth_limit",
            path,
            Some(line),
            "parser.javascript.visit_node",
            "exceeded",
        );
        operation(
            state,
            "parse.truncated_subtree",
            path,
            Some(line),
            "parser.javascript.visit_node",
            "partial_ir",
        );
        let key = crate::identity::path_key(&state.root, path);
        let range = crate::report_authority::point_range(&key, line, 1, start, end);
        loss(state, "depth_truncation", path, Some(range), "parse", None);
        state.fired = true;
        true
    })
}

pub(crate) fn python_node(path: &Path, kind: &str, line: u64, start: u64, end: u64) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if !at_target(state, path, FaultCase::UnsupportedRequiredConstruct)
            || kind != "match_statement"
        {
            return false;
        }
        operation(
            state,
            "parse.visit_match_statement",
            path,
            Some(line),
            "parser.python.visit_node",
            "visited",
        );
        operation(
            state,
            "parse.inject_unsupported_required",
            path,
            Some(line),
            "parser.python.visit_node",
            "unsupported",
        );
        let key = crate::identity::path_key(&state.root, path);
        let range = crate::report_authority::point_range(&key, line, 1, start, end);
        loss(
            state,
            "unsupported_construct",
            path,
            Some(range),
            "parse",
            None,
        );
        state.fired = true;
        true
    })
}

pub(crate) fn import_edge(path: &Path, module: &str, name: &str, line: u64) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        if !at_target(state, path, FaultCase::UnresolvedImport)
            || module != ".peer"
            || name != "value"
        {
            return false;
        }
        operation(
            state,
            "resolve.import_edge",
            path,
            Some(line),
            "graph.resolve_cross_file_imports",
            "examined",
        );
        operation(
            state,
            "resolve.inject_unresolved_import",
            path,
            Some(line),
            "graph.resolve_cross_file_imports",
            "unresolved",
        );
        loss(
            state,
            "unresolved_import",
            path,
            Some(location_range(state, path, line, 1)),
            "resolve",
            None,
        );
        state.fired = true;
        true
    })
}

pub(crate) fn call_edge(location: &Location, caller: &str, callee: &str) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        let path = &location.file;
        if !at_target(state, path, FaultCase::UnresolvedCall)
            || caller != "caller"
            || callee != "target"
        {
            return false;
        }
        operation(
            state,
            "resolve.call_edge",
            path,
            Some(location.line.into()),
            "graph.insert_references",
            "examined",
        );
        operation(
            state,
            "resolve.inject_unresolved_call",
            path,
            Some(location.line.into()),
            "graph.insert_references",
            "unresolved",
        );
        loss(
            state,
            "unresolved_call",
            path,
            Some(location_range(
                state,
                path,
                location.line.into(),
                location.column.into(),
            )),
            "resolve",
            None,
        );
        state.fired = true;
        true
    })
}

pub(crate) fn attribute_edge(location: &Location, origin: &str, name: &str) -> bool {
    STATE.with(|slot| {
        let mut borrow = slot.borrow_mut();
        let Some(state) = borrow.as_mut() else {
            return false;
        };
        let path = &location.file;
        let (case, expected_name, reason, first, second) = if origin == "Annotation" {
            (
                FaultCase::UnresolvedType,
                "Widget",
                "unresolved_type",
                "resolve.type_annotation",
                "resolve.inject_unresolved_type",
            )
        } else {
            (
                FaultCase::UnresolvedReference,
                "peer",
                "unresolved_reference",
                "resolve.value_attribute",
                "resolve.inject_unresolved_reference",
            )
        };
        if !at_target(state, path, case) || name != expected_name {
            return false;
        }
        operation(
            state,
            first,
            path,
            Some(location.line.into()),
            "graph.resolve_import_by_name",
            "examined",
        );
        operation(
            state,
            second,
            path,
            Some(location.line.into()),
            "graph.resolve_import_by_name",
            "unresolved",
        );
        loss(
            state,
            reason,
            path,
            Some(location_range(
                state,
                path,
                location.line.into(),
                location.column.into(),
            )),
            "resolve",
            Some(origin),
        );
        state.fired = true;
        true
    })
}

fn location_range(state: &State, path: &Path, line: u64, column: u64) -> Value {
    let key = crate::identity::path_key(&state.root, path);
    let start = state
        .sources
        .get(path)
        .map(|content| {
            content
                .split_inclusive('\n')
                .take(line.saturating_sub(1) as usize)
                .map(str::len)
                .sum::<usize>() as u64
                + column.saturating_sub(1)
        })
        .unwrap_or(0);
    crate::report_authority::point_range(&key, line, column, start, start.saturating_add(1))
}
