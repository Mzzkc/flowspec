# Lossless report record sharing v1

Structured JSON and YAML reports may carry a root `report_records` object with
`encoding: flowspec.report.records.v1`. This is a versioned wire extension to
report ABI v1 and schema v2. It changes the wire representation of records, not
analysis scope, count, outcome, finding/evidence semantics, or graph meaning.
Consumers must expand the report before interpreting any referenced section.

The public Rust reader is `flowspec::commands::expand_report_records(&Value)`.
`Manifest` / `AnalysisReport` serde deserialization invokes the same reader for
JSON and YAML, including ordinary CLI diff file reading. Unencoded historical
reports remain supported. The graph remains `flowspec.graph.tables.v1` or v2;
`expand_graph_tables` remains the graph reader after report expansion, and both
previous versions remain supported.

The root legacy section names remain present in their established YAML order:
metadata, summary, diagnostics, entities, flows, boundaries, dependency_graph,
type_flows. Seven payload sections use `{"$record": index}`; metadata stays
inline. Authority losses, pattern_state and provenance also use record references, as
do inventory.entries and findings.all. Empty lists remain directly readable.
Scope, inventory counts, finding totals/counts, execution, outcome,
findings_present, gate, producer, display, continuation and report_identity stay
inline. The authority
still has seventeen keys. Every referenced value expands to the exact original
JSON value, including every list element and its original order. This retains
all catalogue entries, semantic evidence, source ranges, input hashes, policies,
capabilities and unresolved debt. No source-padding, filtering, hidden absence,
or false-complete normalization is used.

`report_records` has exactly `encoding`, `shapes`, and `nodes`. `shapes` is a list
of ordered unique object-key arrays. A node is either a scalar JSON value, an
array node `["a", child_index, ...]`, or an object node
`["o", shape_index, value_index, ...]`. Each referenced node precedes its user.
An object node supplies exactly one value per shape key. Scalars retain exact
JSON strings/numbers/booleans/null. Lists never sort; objects use canonical key
ordering. Equal JSON values share a node. Literal arrays or objects resembling
codec markers are encoded as ordinary data nodes and recover unchanged.

Malformed indices, forward/cyclic references, unknown versions, duplicate keys,
shape arity mismatches and missing wrappers are errors. The reader has a 64 MiB
cumulative expanded-node budget to reject hostile amplification. Failure returns
no partial report. This reader limit cannot make an incomplete scope complete.
The producer validates full expansion equality and only chooses sharing when it
reduces the emitted JSON bytes.

The fourteen report-identity inputs remain the complete expanded authority
values and the existing canonical `flowspec/report/v1\0` SHA-256 algorithm.
Expand the report before independently hashing these inputs. Serialization does
not refresh or change the identity. Presentation indices/continuation remain
outside that preimage. JSON/YAML/filter/diagnose callers retain one identity for
equal effective options, sources and configuration.

Generic raw JSON/YAML consumers must explicitly adopt this versioned reader;
raw references are not empty sections. This migration requires independent
consumer judgment before qualification. Existing tests or downstream readers
that assert raw omission or directly index compressed sections are not waived.
