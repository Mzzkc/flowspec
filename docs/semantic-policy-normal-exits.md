# Source-owned normal-exit peer invocation contracts

Explicit or default `.flowspec/config.yaml` configuration can declare:

```yaml
semantic_policy:
  version: 1
  peers:
    - subjects: [source.js::left, source.js::right]
      required_operations: [peer.js::cleanup]
      exit_scope: normal
```

An authority is a project-relative path and exact top-level named function. The
normal-exit relation requires every normal-return control path to invoke each
named operation. Invocation identity follows resolved native Graph call edges,
including original invocation ranges through local import chains. The consumer
expands supported direct callees recursively, preserves branches and early
returns, and refuses loops, recursive calls, unsupported expressions, ambiguous
bindings, and unavailable source. A caller that invokes a different same-name
binding does not satisfy the rule. Literal Boolean branch feasibility is kept.
The finite subset supports blocks, scalar/formal expressions, direct calls,
if/else and returns in Python, JavaScript, TypeScript and Rust.

This is an invocation relation, not a resource cleanup or purity assertion.
Exceptional exits and divergence are explicitly outside `exit_scope: normal`.
Omitting `exit_scope` selects the bounded all-exit extension documented in
[contract-effect-history.md](contract-effect-history.md); unsupported exits retain
explicit incomplete state. Unsupported values and unknown configuration keys are hard config errors.
The effective policy participates in report identity. Unrelated `policy.json`
files have no authority. Layer, migration and public-surface policies retain their explicit incomplete
state here. Declared abstract resource protocols have a separate finite consumer
documented in the extension.

Rust private executable method proofs additionally support a complete selected
single-file scope with a private top-level nominal struct, an explicit inherent
implementation, and no owner use, escape, import/module, macro or registration
route. Public method syntax does not make that private owner externally visible.
The produced executable and private implementation relations have separate
finding identities. Actual typed or associated consumers defeat absence; unknown
owner/trait/registration/multifile closure remains located incomplete. Types alone
are never executable values. Wider private-owner and trait dispatch scopes remain
unimplemented, rather than inheriting a Method/Function partition.
