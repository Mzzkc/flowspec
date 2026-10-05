# Finite source contracts, exit effects and supplied history

The `structured-v2` adapters add original-byte signature, literal operand and
all-exit path facts. Existing normal-return invocation policy remains supported.
These facts feed the public `analyze` and `diagnose_report` authority. They do not
close all thirteen diagnostics or the four-language launch universe.

The validated semantic policy version 1 accepts these additive records:

```yaml
semantic_policy:
  version: 1
  contracts:
    - target: source.js::accept
      parameter_types: [boolean]
  resources:
    - subjects: [source.js::left, source.js::right]
      acquire: source.js::acquire
      release: source.js::release
  history:
    - event_id: supplied-v1-to-v2
      old: source.js::old
      new: source.js::current
      state: shim
```

Authorities name exact project-relative files and top-level functions. Schemas
support Boolean and string parameters, joined to actual native parameter counts
and exact bound call edges. Python, TypeScript and Rust primitive annotations
supply native contracts; JavaScript requires an explicit schema. Native literal
returns also prove contradictions in the three annotated languages. An annotation
never determines an identifier operand's dynamic type. Unknown operands,
unsupported byte/interpolated literals, parameter protocols and shadowed Python
primitive names retain located debt. Static contradiction does not claim compiler
or runtime success, dispatch execution, or a successful trace.

Omitting peer `exit_scope` checks every supported exit. Direct literal throw/raise
syntax is retained, while loops, dynamic effects, handlers, finally, recursion,
Rust panic/unwind and divergent syntax remain unresolved. All-exit callees are
restricted to zero-argument single-literal-return leaves; richer transitive
callees retain debt. The previous normal-exit consumer keeps its broader existing
finite recursive invocation protocol. Neither relation asserts general purity.

Resource policy declares an abstract linear protocol over exact bound endpoint
invocations. Each acquisition owes a later release. Order, duplicate acquisition,
release underflow, branch feasibility and early/exceptional exits are retained.
Endpoint and irrelevant calls must be zero-argument single-literal-return leaves;
unknown callees cannot silently hide a throw or another resource effect. These
are declared abstract effects, not a claim about actual files, locks, sockets,
RAII or operating-system resource ownership. A resource finding requires closed
peer prerequisites and at least one fully satisfied peer. Irrelevant paths with
no acquisition satisfy the invariant. Equal failures retain
`resource_no_satisfied_peer` debt instead of an asymmetric finding.

History is supplied project authority, never inferred from file absence or name
similarity and never synthesized from migration records. A supported `shim`
event needs exact current old/new bindings, an old-to-new call edge, and a live
bound call to the old authority. The supplied event records the historical
identity relation; it is not inferred intent. Findings expose the event and both
bindings. Same-name unrelated declarations and no-history rivals do not qualify.
`moved` and `removed` are accepted transport states but retain explicit old-binding
or transition debt until a snapshot/import identity consumer is constructed.
General value/type/export historical consumers, arbitrary interfaces/schema
objects, dynamic type inference, catch/finally, actual resource implementations
and wider dispatch remain unconstructed. Existing parser/resolution/capability
losses stay visible even when a trustworthy finite positive can be emitted.
