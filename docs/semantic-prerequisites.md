# Shared semantic prerequisites

The ordinary report ABI remains `flowspec.analysis.provenance.v1` (normal ABI1).
All thirteen registered patterns remain enabled. Check authoritative execution,
per-pattern state, counts and findings before interpreting absence. Incomplete
execution can retain proved positive findings. Exit3 is incomplete, never clean.
The compatibility `diagnostics` observations and the qualified `findings.all`
catalogue have deliberately different meanings; a legacy observation is not a
qualified finding merely because it has a pattern name.

## Binding and produced values

Parser-owned semantic facts live on the same losslessly serializable Graph,
separately from compatibility entities. ECMAScript lexical declarations have
scope/binding ordinals, exact initializer/declaration ranges, read and write
uses, and closure state. The currently proved positive subset is an initialized
function-local `let`/`const` scalar literal or a strict `===`/`!==` comparison of
initialized lexical values/literals, with no resolved reads/writes or reflective
access. A returned value, alias, container, registration argument, or captured
read is a consumer. A read of a different shadowed binding is not its consumer.
A getter, coercive comparison, call initializer, destructuring, `var`, or missing
initialization/escape proof cannot receive that positive proof. Python/Rust local
production currently has a ranged `value_binding_unresolved` obligation rather
than a clean negative.

ES imported value/export reads enter the ordinary reference graph through the
same Reference primitive as calls and annotations, with lexical parameter/local
shadowing respected. Type origins stay separate from runtime Read origins.
Rust standalone functions retain declared visibility and expose transitive
`Graph::effective_visibility`: `pub` inside a private inline module is private
unless consumed/re-exported. Source calls, value references and re-exports
prevent absence. Macro/attribute and trait/associated/framework registration
closure remains an explicit prerequisite, not an unread-name heuristic.

## Exact duplication rule

[duplication-normalization-v1](duplication-normalization-v1.md) is the exact public
normative rule, SHA256
`8da06b2a92262e831d20a728d3f9fc8c62853b39e727009e16871a69f07f05a8`.
The independently admitted rule defines v1; this implementation does not expand
its domain. Function bodies compare ordered binding-aware trees and scope,
binding, type, effect and control protocol tables. Exact W>=8 and shape tests
apply. Every equal unordered pair is a finding, including three findings for
three equal functions. Names alone, hashes alone and shared utility callees
cannot prove equivalence. Semantic IDs use canonical rule JSON and same-name
source declaration ordinals. Evidence retains rule digest, input/range bindings,
normal forms, binder correspondence and threshold witnesses.

The current complete normalizer subset covers ordinary named Python and
JavaScript functions with simple parameters, scalar reads/literals, identifier
assignments/lexical declarations, supported unary/binary operations, ordered
blocks, if/while, simple break/continue and returns. It preserves declaration
kind, operand order, keyword-visible Python parameter names, literal value and
initialization state. Python methods, nested functions, decorators, generators,
async functions, direct calls without stable effect proof, exceptional syntax,
unsupported literal decoders and unresolved fields have located `DUP_*` reasons.
Native TypeScript static inference and Rust type/ownership normalization remain
held. A proved pair is retained beside an unsupported subject.

Exact language environments must enter through the explicit policy below.
Currently implemented environments are `Python/3.14` targeting `Python` and
`ECMA-262/2025` targeting `ECMAScript`. An absent or unsupported environment yields
`DUP_ENVIRONMENT_UNRESOLVED`; a supplied arbitrary string is not a protocol proof.
The compatibility duplication detector remains a named coarse observation
surface and does not define authoritative duplication.

## Explicit user-owned policy transport

Only `.flowspec/config.yaml` or the actual `--config PATH`/`Config::load` path is
configuration. No arbitrary file named `policy.json` is read as policy. This
validated YAML is included in effective-config provenance and its digest:

```yaml
semantic_policy:
  version: 1
  environments:
    javascript:
      version: ECMA-262/2025
      target: ECMAScript
  layers:
    - from: application/**
      to: infrastructure/**
  peers:
    - subjects: [source.js::first, source.js::second]
      required_operations: [resources.js::cleanup]
  migrations:
    - old: old.js::authority
      new: new.js::authority
  exports:
    - module: package/index.js
      symbols: [RequiredApi]
```

`layers` declares forbidden project-relative module-path glob edges. Peer
subjects are exact `project-relative-path::declaration` authorities, with at
least two distinct subjects and at least one required operation. Migration
old/new authorities must differ. Export policies require a module and nonempty
symbol list. Unknown fields, unsupported policy versions, malformed globs,
absolute/escaping authority paths and invalid contracts are hard config errors.
A Rust environment additionally requires `overflow: checked` or `wrapping`;
transporting it does not implement its normalizer.

Transport is implemented; qualified complete CFG/effect/resource, migration,
layer-edge and export-graph policy evaluation is not yet implemented. A requested
operation receives `policy_operation_unavailable` affecting its actual pattern,
with the validated policy retained. An absent layer policy is visibly
not_applicable. Do not classify unsupported policy transport/evaluation as an
ordinary default-config false negative, or as a passing policy diagnostic.

## Remaining launch operations

This component supplies lexical value/import/visibility prerequisites and a
finite duplication subset. Exact component tests are not the thirteen-family,
four-language acceptance universe. Isolated-cluster entry/dispatch closure,
general executable/value escape closure, peer CFG/resource invariants, trait and
framework dispatch, static type/serialization contradictions, complete module
SCC authority, user layer evaluation, migration authority joins, moved-reference
proof, import side-effect closure and parent export-policy evaluation still
require implementation and independent current-source qualification. Final
integration, current all-component judgment, the one unfiltered whole, coverage,
oracle/mutants, reproducibility, installed-product and release gates remain
separate. This component confers no launch or publication acceptance.
