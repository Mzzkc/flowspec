# Trace Expectations

Expected cross-file trace paths keyed by `symbol`, `direction`, and edge kind, following
the schema in [`../SCHEMA.md`](../SCHEMA.md). The comparator rule `R005_TRACE_MISSING_PATH`
fails when the actual trace output lacks an expected hop or edge kind.

## Wave 1A status: intentionally empty

Trace expectations are Wave 1B work. They depend on the trace fixtures
(`tests/fixtures/{python,js,rust}/cross_file/`) and the v1 trace contract being locked
first (plan item #6). The validator's `--self-test` mode already exercises the
trace-expectation shape against
[`../_selftest/good/traces/planted.yaml`](../_selftest/good/traces/planted.yaml), so the
validation path is proven without live fixtures here.

## Wave 1B entry plan

Add one YAML per required trace assertion (the plan's oracle command shape), at minimum:

- Python: `format_output` backward (`tests/fixtures/python/cross_file/flow_trace`)
- JavaScript/TypeScript: `helper` backward (`tests/fixtures/js/cross_file/esm_named`)
- Rust: `helper` backward (`tests/fixtures/rust/cross_file`)
- Rust: `entry_point` forward — proves `lib.rs::entry_point -> handler.rs::handle ->
  utils.rs::helper`, closing the zero-flow failure from `01-recon.md`.

Each entry must list `expected_hops` with `entity` and `edge_kind` per hop; valid edge
kinds are the graph edges flowspec can prove (see `SCHEMA.md §2`). If the trace CLI
cannot prove an edge kind for v1, the CLI/help/spec are narrowed to the subset it can
prove — no public text may promise "everything touching the symbol" unless the oracle
proves the full edge set.
