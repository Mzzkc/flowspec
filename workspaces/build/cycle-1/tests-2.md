# Test Specification — QA 2 (QA-Analysis)
## Cycle 1 | Paired with Worker 2 (Sentinel)

---

## Scope

This spec covers the test triad (true-positive, true-negative, adversarial) for both new diagnostic patterns (`duplication`, `asymmetric_handling`) and validation tests for the four manifest wiring tasks (boundaries, flow types, key_flows, exit_points). All tests are TDD — written before implementation exists.

Every test builds a real `Graph` using the existing `test_utils` helpers (`make_symbol`, `make_import`, `add_ref`, etc.) and exercises the actual pattern `detect()` function or manifest assembly code. No mocking the graph.

---

## 1. Duplication Pattern Tests

### Interface Contract

The implementation will live at `flowspec/src/analyzer/patterns/duplication.rs` and export:

```rust
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic>
```

It will be registered in `patterns/mod.rs` with `DiagnosticPattern::Duplication`.

### Test 1.1: True Positive — Identical Callees with Matching Arity

```
TEST: test_duplication_true_positive_identical_callees
GIVEN: Two functions in the same file ("handlers.py") with matching arity (2 params each)
       and identical callees sets {validate, save, notify}
       - process_order(order, ctx) calls validate, save, notify
       - process_refund(refund, ctx) calls validate, save, notify
WHEN:  duplication::detect(graph, project_root) is called
THEN:  At least one Diagnostic is returned with:
       - pattern == DiagnosticPattern::Duplication
       - severity == Severity::Warning
       - confidence == Confidence::Moderate
       - entity contains both function names (or one cites the other in evidence)
       - evidence is non-empty with at least one observation mentioning similarity
       - location is non-empty (file:line format)
EDGE:  This is the canonical case — two sibling functions doing the same structural work.
       If this doesn't fire, the pattern is broken. Jaccard similarity = 1.0 (100%).
```

**Graph construction:**
```rust
let mut g = Graph::new();
let f = "handlers.py";
let validate = g.add_symbol(make_symbol("validate", Function, Private, f, 5));
let save = g.add_symbol(make_symbol("save", Function, Private, f, 10));
let notify = g.add_symbol(make_symbol("notify", Function, Private, f, 15));

let mut process_order = make_symbol("process_order", Function, Public, f, 20);
process_order.signature = Some("(order: Order, ctx: Context) -> Result".to_string());
let po_id = g.add_symbol(process_order);

let mut process_refund = make_symbol("process_refund", Function, Public, f, 30);
process_refund.signature = Some("(refund: Refund, ctx: Context) -> Result".to_string());
let pr_id = g.add_symbol(process_refund);

// Both call the same 3 functions
for caller in [po_id, pr_id] {
    add_ref(&mut g, caller, validate, ReferenceKind::Call, f);
    add_ref(&mut g, caller, save, ReferenceKind::Call, f);
    add_ref(&mut g, caller, notify, ReferenceKind::Call, f);
}
```

### Test 1.2: True Negative — Same Name Different Structure

```
TEST: test_duplication_true_negative_different_callees
GIVEN: Two functions in the same file with same name prefix but completely different callees
       - parse_json(data) calls json_decode, validate_schema
       - parse_xml(data) calls xml_decode, transform_tree, normalize
WHEN:  duplication::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::Duplication is returned
       (zero overlap between callees sets, Jaccard = 0.0)
EDGE:  Functions with similar names but different behavior must NOT trigger.
       This catches implementations that use name-based heuristics instead of structural analysis.
```

**Graph construction:** Two functions in "parsers.py". `parse_json` calls `json_decode` + `validate_schema`. `parse_xml` calls `xml_decode` + `transform_tree` + `normalize`. Zero intersection in callees.

### Test 1.3: Adversarial — Threshold Boundary (70% Jaccard)

```
TEST: test_duplication_adversarial_threshold_boundary
GIVEN: Two functions sharing exactly 70% callees overlap:
       - handler_a calls {log, validate, save, notify, format_a, audit, check}
         (7 callees)
       - handler_b calls {log, validate, save, notify, format_b, audit, check}
         (7 callees)
       Intersection = {log, validate, save, notify, audit, check} = 6
       Union = {log, validate, save, notify, format_a, format_b, audit, check} = 8
       Jaccard = 6/8 = 0.75 (above 0.7 threshold) — should trigger
WHEN:  duplication::detect(graph, project_root) is called
THEN:  A Diagnostic with pattern == DiagnosticPattern::Duplication IS returned
EDGE:  Tests the boundary condition of the similarity threshold.
```

```
TEST: test_duplication_adversarial_below_threshold
GIVEN: Two functions sharing exactly 60% callees overlap:
       - handler_c calls {log, validate, save, format_c, cache}
         (5 callees)
       - handler_d calls {log, validate, transform, render, send}
         (5 callees)
       Intersection = {log, validate} = 2
       Union = {log, validate, save, format_c, cache, transform, render, send} = 8
       Jaccard = 2/8 = 0.25 — well below threshold
WHEN:  duplication::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::Duplication is returned
EDGE:  Functions that share utility callees (log, validate) but serve different purposes
       must not trigger. This is the most common false-positive scenario.
```

### Test 1.4: Adversarial — Zero Callees Functions

```
TEST: test_duplication_adversarial_zero_callees
GIVEN: Two functions in the same file with zero callees each
       - get_name() returns a string literal, calls nothing
       - get_version() returns a string literal, calls nothing
WHEN:  duplication::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::Duplication is returned
EDGE:  Empty callees sets produce Jaccard = 0/0 (undefined). The implementation must
       handle this gracefully — either skip or treat as 0.0. Two leaf functions calling
       nothing have no structural evidence of duplication.
```

### Test 1.5: Adversarial — Different Arity Should Not Match

```
TEST: test_duplication_adversarial_different_arity
GIVEN: Two functions with identical callees but different arity
       - setup_user(config, db, logger, flags) calls validate, save, notify (arity 4)
       - handle_event(event) calls validate, save, notify (arity 1)
WHEN:  duplication::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::Duplication is returned
EDGE:  Per investigation spec, matching arity is required alongside callees overlap.
       A setup function and an event handler sharing infrastructure callees is normal,
       not duplication. The arity mismatch (4 vs 1) signals different roles.
```

**Graph construction:** Set `signature` on both symbols: `"(config, db, logger, flags) -> None"` vs `"(event) -> None"`. Arity difference should prevent detection.

### Test 1.6: Adversarial — Test File Exclusion

```
TEST: test_duplication_adversarial_test_file_excluded
GIVEN: Two functions with identical callees in a test file ("test_handlers.py")
WHEN:  duplication::detect(graph, project_root) is called
THEN:  No Diagnostic returned — test files are excluded per exclusion.rs rules
EDGE:  Test helpers frequently have identical structure (setup, act, assert pattern).
       Flagging test duplication would be pure noise.
```

### Test 1.7: Adversarial — Self-Recursive Function

```
TEST: test_duplication_adversarial_self_recursive
GIVEN: Two functions where one calls itself recursively plus shared callees:
       - traverse(node) calls traverse (self), process, validate (callees: {traverse, process, validate})
       - handle(item) calls process, validate (callees: {process, validate})
       If self-calls NOT excluded: Jaccard({traverse,process,validate}, {process,validate}) = 2/3 = 0.67 (below threshold)
       If self-calls excluded: Jaccard({process,validate}, {process,validate}) = 1.0 (above threshold)
WHEN:  duplication::detect(graph, project_root) is called
THEN:  Behavior depends on whether self-calls are excluded from callees set.
       Per investigation: self-recursive calls SHOULD be excluded. So with matching arity,
       this SHOULD trigger duplication.
EDGE:  Tests self-call exclusion — a critical edge case for recursive functions.
```

### Test 1.8: True Positive — Single Shared Callee with Matching Arity

```
TEST: test_duplication_true_positive_single_callee_match
GIVEN: Two functions that each call exactly one function, and it's the same function:
       - serialize_user(user) calls to_json
       - serialize_order(order) calls to_json
       Both have arity 1, same signature shape.
WHEN:  duplication::detect(graph, project_root) is called
THEN:  A Diagnostic with pattern == DiagnosticPattern::Duplication IS returned.
       Jaccard = 1/1 = 1.0 with matching arity. Per investigation: "If the callee matches
       AND arity matches, this should trigger."
EDGE:  Single-callee pairs are a real duplication signal — wrapper functions that should
       likely be generified.
```

---

## 2. Asymmetric Handling Pattern Tests

### Interface Contract

The implementation will live at `flowspec/src/analyzer/patterns/asymmetric_handling.rs` and export:

```rust
pub fn detect(graph: &Graph, project_root: &Path) -> Vec<Diagnostic>
```

### Test 2.1: True Positive — One Sibling Missing Consensus Callee

```
TEST: test_asymmetric_true_positive_missing_validate
GIVEN: 4 functions in the same file ("api/handlers.py"), same kind (Function), similar arity (2):
       - handle_create(req, ctx) calls [validate, authorize, save, respond]
       - handle_update(req, ctx) calls [validate, authorize, update, respond]
       - handle_delete(req, ctx) calls [authorize, delete, respond]  ← MISSING validate
       - handle_read(req, ctx) calls [validate, authorize, fetch, respond]
       Consensus callees (present in ≥3 of 4): {validate, authorize, respond}
       handle_delete is missing `validate` — consensus callee absent
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  At least one Diagnostic is returned with:
       - pattern == DiagnosticPattern::AsymmetricHandling
       - severity == Severity::Warning
       - confidence == Confidence::Moderate
       - entity mentions handle_delete (or the evidence does)
       - evidence mentions the missing callee(s)
EDGE:  The canonical case — an API handler missing validation that all siblings perform.
       This is the exact scenario from the spec: "One API handler validates input, another
       doing similar work doesn't."
```

**Graph construction:**
```rust
let mut g = Graph::new();
let f = "api/handlers.py";

// Shared callees
let validate = g.add_symbol(make_symbol("validate", Function, Private, f, 5));
let authorize = g.add_symbol(make_symbol("authorize", Function, Private, f, 10));
let save = g.add_symbol(make_symbol("save", Function, Private, f, 15));
let update = g.add_symbol(make_symbol("update", Function, Private, f, 20));
let delete = g.add_symbol(make_symbol("delete", Function, Private, f, 25));
let fetch = g.add_symbol(make_symbol("fetch", Function, Private, f, 30));
let respond = g.add_symbol(make_symbol("respond", Function, Private, f, 35));

// Handlers with signatures
let mut hc = make_symbol("handle_create", Function, Public, f, 40);
hc.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
let hc_id = g.add_symbol(hc);

let mut hu = make_symbol("handle_update", Function, Public, f, 50);
hu.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
let hu_id = g.add_symbol(hu);

let mut hd = make_symbol("handle_delete", Function, Public, f, 60);
hd.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
let hd_id = g.add_symbol(hd);

let mut hr = make_symbol("handle_read", Function, Public, f, 70);
hr.signature = Some("(req: Request, ctx: Context) -> Response".to_string());
let hr_id = g.add_symbol(hr);

// Wire calls
add_ref(&mut g, hc_id, validate, Call, f);
add_ref(&mut g, hc_id, authorize, Call, f);
add_ref(&mut g, hc_id, save, Call, f);
add_ref(&mut g, hc_id, respond, Call, f);

add_ref(&mut g, hu_id, validate, Call, f);
add_ref(&mut g, hu_id, authorize, Call, f);
add_ref(&mut g, hu_id, update, Call, f);
add_ref(&mut g, hu_id, respond, Call, f);

// handle_delete — MISSING validate
add_ref(&mut g, hd_id, authorize, Call, f);
add_ref(&mut g, hd_id, delete, Call, f);
add_ref(&mut g, hd_id, respond, Call, f);

add_ref(&mut g, hr_id, validate, Call, f);
add_ref(&mut g, hr_id, authorize, Call, f);
add_ref(&mut g, hr_id, fetch, Call, f);
add_ref(&mut g, hr_id, respond, Call, f);
```

### Test 2.2: True Negative — Intentionally Different Functions

```
TEST: test_asymmetric_true_negative_different_roles
GIVEN: 3 functions in the same file with same kind (Function) but different roles:
       - setup_db(config) calls [connect, migrate, seed] (arity 1, setup role)
       - handle_request(req, ctx) calls [validate, process, respond] (arity 2, handler role)
       - cleanup(resources) calls [close, flush, deallocate] (arity 1, cleanup role)
       No meaningful callee overlap — these serve completely different purposes.
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::AsymmetricHandling is returned
EDGE:  Functions in the same file that serve different architectural roles must NOT be
       grouped as siblings. The arity filter (setup=1, handler=2, cleanup=1) and zero
       callees overlap should prevent grouping.
```

### Test 2.3: Adversarial — Group of 2 (Below Minimum)

```
TEST: test_asymmetric_adversarial_group_too_small
GIVEN: 2 functions in the same file with identical arity and one missing a callee:
       - process_a(data, config) calls [validate, transform, save]
       - process_b(data, config) calls [transform, save]  ← missing validate
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::AsymmetricHandling is returned
EDGE:  Per investigation spec, groups of 2 are "too small for meaningful consensus."
       Minimum group size is 3. Two functions where one does less is ambiguous — it could
       be intentional specialization, not asymmetry.
```

### Test 2.4: Adversarial — Cross-Module Grouping Should Not Happen

```
TEST: test_asymmetric_adversarial_cross_module_no_grouping
GIVEN: 3 functions with matching arity and kind, but in DIFFERENT files:
       - "auth/handlers.py": handle_login(req, ctx) calls [validate, authenticate, respond]
       - "user/handlers.py": handle_register(req, ctx) calls [validate, create, respond]
       - "payment/handlers.py": handle_charge(req, ctx) calls [create, respond]  ← missing validate
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::AsymmetricHandling is returned
EDGE:  Per investigation: "I'll restrict to same-file for v1." Cross-module functions
       serving different domains should NOT be compared as siblings even when they share
       callees. A payment handler not calling `validate` is a different design from an
       auth handler not calling `validate`.
```

### Test 2.5: Adversarial — All Siblings Identical (No Asymmetry)

```
TEST: test_asymmetric_adversarial_all_identical_no_finding
GIVEN: 4 functions in the same file, all with identical callees:
       - handler_a calls [validate, process, respond]
       - handler_b calls [validate, process, respond]
       - handler_c calls [validate, process, respond]
       - handler_d calls [validate, process, respond]
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  No Diagnostic with pattern == DiagnosticPattern::AsymmetricHandling is returned
EDGE:  If all siblings are identical, there is no asymmetry to report. (This would be
       a duplication finding, not an asymmetric handling finding.)
```

### Test 2.6: Adversarial — Mixed Kind Should Not Group

```
TEST: test_asymmetric_adversarial_mixed_kind_no_grouping
GIVEN: 3 symbols in the same file, same arity, but mixed kinds:
       - process (Function, arity 2) calls [validate, save]
       - handle (Function, arity 2) calls [validate, respond]
       - run (Method, arity 2) calls [respond]  ← missing validate
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  `run` (a Method) is NOT grouped with `process` and `handle` (Functions).
       The grouping requires same SymbolKind. So only {process, handle} form a group,
       but that's only 2 — below minimum. No finding.
EDGE:  Mixing Functions and Methods in the same group would produce noise — methods
       have implicit `self` context that changes their behavioral expectations.
```

### Test 2.7: True Positive — Consensus Threshold with 5 Siblings

```
TEST: test_asymmetric_true_positive_two_missing_callees
GIVEN: 5 handler functions in the same file, same kind (Function), arity 2:
       - h1 calls [auth, validate, log, process, respond]
       - h2 calls [auth, validate, log, transform, respond]
       - h3 calls [auth, validate, log, fetch, respond]
       - h4 calls [auth, log, delete, respond]  ← missing validate
       - h5 calls [auth, validate, log, create, respond]
       Consensus (≥4 of 5): {auth, validate, log, respond}
       h4 is missing `validate` from the consensus set
WHEN:  asymmetric_handling::detect(graph, project_root) is called
THEN:  Diagnostic returned citing h4 for missing `validate`
EDGE:  Tests consensus threshold calculation with a larger group. 4/5 = 80%
       which should count as consensus.
```

---

## 3. Pattern Registration Tests

### Test 3.1: Both Patterns Registered in Registry

```
TEST: test_registry_includes_duplication_and_asymmetric
GIVEN: A graph with planted duplication (from test 1.1 graph) and asymmetry (from test 2.1 graph)
WHEN:  patterns::run_all_patterns(graph, project_root) is called
THEN:  The returned Vec<Diagnostic> includes findings with:
       - At least one with pattern == DiagnosticPattern::Duplication
       - At least one with pattern == DiagnosticPattern::AsymmetricHandling
EDGE:  Ensures both patterns are actually wired into the registry at patterns/mod.rs.
       A pattern that exists as a module but isn't registered is dead code.
```

### Test 3.2: Pattern Filter Works for New Patterns

```
TEST: test_filter_includes_new_patterns
GIVEN: A graph with planted issues for both new patterns
WHEN:  run_patterns(graph, &filter_with_only_duplication, project_root) is called
THEN:  Only Duplication findings returned, no AsymmetricHandling
WHEN:  run_patterns(graph, &filter_with_only_asymmetric, project_root) is called
THEN:  Only AsymmetricHandling findings returned, no Duplication
EDGE:  Verifies the PatternFilter correctly filters by the new DiagnosticPattern variants.
```

---

## 4. Boundary Manifest Wiring Tests

Worker 2 will derive module boundaries from cross-file edges in `analyzer/extraction.rs` and wire them to `lib.rs:613`.

### Test 4.1: Non-Empty Boundaries After Cross-File Analysis

```
TEST: test_boundary_wiring_produces_entries
GIVEN: A graph with cross-file references:
       - "src/main.py": main() calls process() in "src/processor.py"
       - "src/processor.py": process() calls validate() in "src/validator.py"
       Three files, two cross-file call edges = at least 1 module boundary
WHEN:  The boundary extraction function is called on this graph
THEN:  The resulting Vec<BoundaryEntry> is non-empty, and each entry has:
       - id: non-empty string
       - boundary_type: "module"
       - from: file/module path of the caller
       - to: file/module path of the callee
       - crossing_points: at least one CrossingPoint with non-empty func name
EDGE:  VERIFIED: No language adapter produces Boundary objects. Worker 2 is deriving
       boundaries from cross-file edges. This test validates that approach works.
```

**Graph construction:**
```rust
let mut g = Graph::new();
let main_file = "src/main.py";
let proc_file = "src/processor.py";
let val_file = "src/validator.py";

let main_fn = g.add_symbol(make_symbol("main", Function, Public, main_file, 1));
let process_fn = g.add_symbol(make_symbol("process", Function, Public, proc_file, 1));
let validate_fn = g.add_symbol(make_symbol("validate", Function, Public, val_file, 1));

// Cross-file calls
add_ref(&mut g, main_fn, process_fn, ReferenceKind::Call, main_file);
add_ref(&mut g, process_fn, validate_fn, ReferenceKind::Call, proc_file);
```

### Test 4.2: No Boundaries for Same-File References

```
TEST: test_boundary_wiring_no_intra_file_boundaries
GIVEN: A graph with only same-file references:
       - "utils.py": helper_a() calls helper_b(), helper_b() calls helper_c()
WHEN:  The boundary extraction function is called on this graph
THEN:  The resulting Vec<BoundaryEntry> is empty
EDGE:  Same-file calls are NOT module boundaries. Intra-module communication
       should produce zero boundary entries.
```

### Test 4.3: Boundary Deduplication

```
TEST: test_boundary_wiring_deduplicates_same_module_pair
GIVEN: A graph where file A has 3 different functions that all call functions in file B:
       - "a.py": fn1 calls "b.py":helper, fn2 calls "b.py":helper, fn3 calls "b.py":util
WHEN:  The boundary extraction function is called
THEN:  Only ONE BoundaryEntry for the a.py → b.py boundary, but with multiple
       crossing_points listing all the functions that cross it
EDGE:  Multiple references between the same two files should produce one boundary
       with multiple crossing points, not N duplicate boundaries.
```

---

## 5. Flow Type Tracking Tests

Worker 2 will thread `Symbol.signature` through `FlowStep.in_type`/`out_type` at `lib.rs:514-518`.

### Test 5.1: Typed Functions Produce Non-Unknown Flow Steps

```
TEST: test_flow_type_tracking_uses_signature
GIVEN: A graph where functions have signatures:
       - main() with signature "(args: list) -> int" calls process()
       - process() with signature "(data: dict) -> Result" calls save()
       - save() with signature "(item: dict) -> bool"
       Flow path: main → process → save
WHEN:  The flow assembly produces FlowStep entries for this path
THEN:  FlowStep for process has:
       - in_type containing type info (not "unknown") — e.g., "dict" from parameter
       - out_type containing type info (not "unknown") — e.g., "Result" from return
       FlowStep for save has meaningful in_type/out_type derived from its signature
EDGE:  VERIFIED at lib.rs:517-518: currently hardcoded "unknown". Worker 2 will use
       Symbol.signature (populated by all 3 adapters per investigation) to fill real types.
```

### Test 5.2: Functions Without Signatures Keep "unknown"

```
TEST: test_flow_type_tracking_graceful_fallback
GIVEN: A graph where one function has no signature (signature: None):
       - main() with signature "()" calls helper()
       - helper() with signature: None (no type info)
WHEN:  The flow assembly produces FlowStep entries
THEN:  FlowStep for helper has in_type == "unknown" and out_type == "unknown"
       (graceful fallback, not a panic or error)
EDGE:  Not every symbol has a signature (e.g., Variables, some adapters skip certain patterns).
       The implementation must handle None gracefully.
```

---

## 6. Summary Key Flows Tests

Worker 2 will populate `summary.key_flows` at `lib.rs:602` from flow analysis results.

### Test 6.1: Non-Empty Key Flows After Analysis

```
TEST: test_key_flows_populated_from_flow_paths
GIVEN: A graph with multiple flow paths of different lengths:
       - Path A: entry → step1 → step2 → step3 → exit (4 steps)
       - Path B: entry → step1 (1 step)
       - Path C: entry → step1 → step2 (2 steps)
WHEN:  The manifest assembly populates summary.key_flows
THEN:  key_flows is non-empty (at least 1 entry)
       Each KeyFlow has:
       - name: non-empty string
       - path_summary: non-empty string describing the flow
       The longest/most significant flows appear (Path A should be represented)
EDGE:  VERIFIED at lib.rs:602: currently hardcoded Vec::new(). Worker 2 will derive
       from flow_paths, selecting top N by step count.
```

### Test 6.2: Empty Key Flows When No Flows Exist

```
TEST: test_key_flows_empty_when_no_flows
GIVEN: A graph with only isolated symbols (no call edges, no flow paths)
WHEN:  The manifest assembly populates summary.key_flows
THEN:  key_flows is an empty Vec (not an error, not a panic)
EDGE:  Edge case — a project with no connected call graph should produce empty key_flows,
       not crash.
```

---

## 7. Summary Exit Points Tests

Worker 2 will populate `summary.exit_points` at `lib.rs:601` by querying for public symbols that are terminal in the call graph.

### Test 7.1: Non-Empty Exit Points for Public API

```
TEST: test_exit_points_populated_for_public_leaf_functions
GIVEN: A graph with:
       - public_api(req) — Public, has callers but NO callees (terminal node)
       - internal_helper() — Private, has callers and callees
       - main() — Public, entry point (has callees, no callers)
WHEN:  The manifest assembly populates summary.exit_points
THEN:  exit_points contains "public_api" (or its qualified name)
       exit_points does NOT contain "internal_helper" (private)
       exit_points does NOT contain "main" (it has callees — it's an entry, not an exit)
EDGE:  VERIFIED at lib.rs:601: currently hardcoded Vec::new(). Worker 2 will query for
       public symbols with empty callees (data leaves the system through them).
```

### Test 7.2: Exit Points Exclude Private Functions

```
TEST: test_exit_points_exclude_private
GIVEN: A graph where a private function is a leaf (zero callees):
       - _internal_sink(data) — Private, zero callees
       - export_data(data) — Public, zero callees
WHEN:  The manifest assembly populates summary.exit_points
THEN:  exit_points contains "export_data" but NOT "_internal_sink"
EDGE:  Private leaf functions are internal sinks, not system exit points.
       Only public API surfaces count as exits.
```

### Test 7.3: Empty Exit Points When All Functions Are Internal

```
TEST: test_exit_points_empty_when_all_private
GIVEN: A graph with only private functions (no public API surface)
WHEN:  The manifest assembly populates summary.exit_points
THEN:  exit_points is an empty Vec
EDGE:  A purely internal module with no public interface has no exit points.
```

---

## 8. Confidence Calibration Tests

### Test 8.1: Both New Patterns Report Moderate Confidence

```
TEST: test_new_patterns_confidence_is_moderate
GIVEN: Graphs from tests 1.1 (duplication) and 2.1 (asymmetric_handling)
WHEN:  Each pattern's detect() function returns findings
THEN:  ALL returned diagnostics have confidence == Confidence::Moderate
EDGE:  Per spec in diagnostics.yaml: both duplication and asymmetric_handling have
       inherently moderate confidence. A finding with High confidence from these patterns
       would be a bug — structural similarity is always a heuristic, never structural proof.
```

### Test 8.2: Evidence Quality — Every Finding Has Concrete Evidence

```
TEST: test_new_patterns_evidence_is_concrete
GIVEN: Graphs from tests 1.1 and 2.1
WHEN:  Each pattern's detect() function returns findings
THEN:  Every Diagnostic has:
       - evidence.len() >= 1
       - At least one evidence item with a non-empty observation
       - observation contains specific data (function names, counts, percentages) —
         NOT vague phrases like "may be duplicated"
EDGE:  Evidence quality is a MUST per diagnostic.rs doc comment: "Diagnostics with missing
       evidence or vague suggestions are considered broken."
```

---

## 9. Integration: Clean Code Graph Produces Zero New-Pattern Findings

```
TEST: test_clean_code_no_new_pattern_findings
GIVEN: The existing build_clean_code_graph() from test_utils.rs
       (all functions connected, all imports used, well-structured)
WHEN:  run_all_patterns(graph, project_root) is called
THEN:  No Diagnostic with pattern == Duplication or AsymmetricHandling is returned
EDGE:  Clean, well-structured code must produce zero false positives from the new patterns.
       This is the most important calibration test — if clean_code_graph triggers either
       new pattern, confidence calibration is broken.
```

---

## Implementation Notes for Worker 2

1. **Test file location:** Tests should be added as `#[cfg(test)] mod tests` inside `duplication.rs` and `asymmetric_handling.rs` respectively, plus integration tests in `patterns/mod.rs::tests`.

2. **Graph builders for new patterns:** Consider adding `build_duplication_graph()` and `build_asymmetric_handling_graph()` to `test_utils.rs` following the existing pattern (e.g., `build_dead_code_graph()`). The graph constructions above are canonical fixtures.

3. **Signature parsing:** For `in_type`/`out_type` extraction from `Symbol.signature`, a simple `split("->")` approach works for the `"(param: Type) -> ReturnType"` format. The left side gives input types, right side gives return type. Handle `None` signatures by falling back to `"unknown"`.

4. **Boundary extraction function signature suggestion:**
   ```rust
   pub fn extract_boundaries(graph: &Graph, project_root: &Path) -> Vec<BoundaryEntry>
   ```
   This belongs in `analyzer/extraction.rs` alongside the existing `extract_dependency_graph()`.

5. **Key flows selection:** Top 5 by step count per investigation decision. Tests validate non-emptiness and structure, not exact ranking — that's an implementation detail.

---

## Test Count Summary

| Category | Tests | True Positive | True Negative | Adversarial |
|----------|-------|---------------|---------------|-------------|
| Duplication | 8 | 2 (1.1, 1.8) | 1 (1.2) | 5 (1.3a, 1.3b, 1.4, 1.5, 1.6, 1.7) |
| Asymmetric Handling | 7 | 2 (2.1, 2.7) | 1 (2.2) | 4 (2.3, 2.4, 2.5, 2.6) |
| Registration | 2 | — | — | — |
| Boundary Wiring | 3 | 1 (4.1) | 1 (4.2) | 1 (4.3) |
| Flow Types | 2 | 1 (5.1) | — | 1 (5.2) |
| Key Flows | 2 | 1 (6.1) | — | 1 (6.2) |
| Exit Points | 3 | 1 (7.1) | — | 2 (7.2, 7.3) |
| Confidence | 2 | — | — | 2 (8.1, 8.2) |
| Integration | 1 | — | 1 (9) | — |
| **Total** | **30** | **8** | **4** | **18** |

18 of 30 tests are adversarial or edge-case focused. If every test passes on first try, the implementation isn't being tested hard enough.

---

## Personal Notes

This is my first cycle. Reading the investigation report from Sentinel (Worker 2), the boundary gap finding changed my entire approach to section 4. My initial assumption was that boundary tests would verify data flowing through the existing `Boundary` IR type and the graph's `boundaries` SlotMap. Wrong. The pipeline is dry — no adapter produces boundary data. Sentinel's pragmatic approach (derive from cross-file edges) means my boundary tests need to validate a brand-new extraction pathway, not an existing one.

The asymmetric handling pattern feels like the hardest thing to test well. The grouping heuristic (same file + same kind + similar arity) is inherently a judgment call, and my adversarial tests are designed to probe the boundaries of that judgment. Test 2.6 (mixed kinds) and test 2.3 (group of 2) are the ones I expect to be most load-bearing — they catch the two most common false-positive pathways.

The duplication threshold boundary tests (1.3a/1.3b) are the ones I'm most satisfied with. Testing exact Jaccard boundaries with carefully computed set overlaps means any off-by-one in the threshold comparison will fail. The 7-callee and 5-callee constructions were chosen so the Jaccard fractions are clean and unambiguous.

What I'm watching: whether the self-recursive exclusion (test 1.7) is handled. This is a subtle edge case that could easily be missed in implementation. If `callees()` returns the function itself (self-recursive call), and the implementation doesn't filter self-IDs from the callees set, then every recursive function will have an artificially inflated callees set that suppresses duplication detection.

Down. Forward. Through.

*— QA-Analysis (QA 2), Cycle 1 Test Design*
