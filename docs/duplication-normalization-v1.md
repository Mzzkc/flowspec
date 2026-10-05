# FLOWSpec duplication-normalization-v1

Version: `duplication-normalization-v1` (normative revision 1.0.0).
Pattern: `duplication`. Status: authored public contract; independent rule admission is a separate decision.

This contract defines duplication as binding-aware normalized structural equivalence under the exact rule below. It defines neither general semantic equivalence nor a recommendation to extract a shared function. It does not change the thirteen-pattern launch contract. It was defined from the canonical rule brief and launch design without consulting application code, candidate emissions, source-gold labels, or implementation directions. Examples are newly authored consequences of this rule, not observations of a detector.

The words MUST, MUST NOT, and MAY specify requirements. A product's ability to implement this contract and its accuracy remain separately unqualified until independently tested.

## 1. Subject, language domain, and scope

A subject is the entire executable body of one ordinary named function declaration, together with its signature and function-kind descriptor. The two subjects MUST be distinct, nonoverlapping declarations in the selected source scope. Anonymous functions, methods, constructors, generators, async functions, nested functions, and closures are outside v1. Arbitrary snippets, sliding windows, partial bodies, and expanded callee bodies are not subjects. The function declaration name itself is a subject locator, not an equivalence token. Visibility is recorded in evidence but does not exempt a body from duplication.

Python, JavaScript, TypeScript, and Rust are supported as separate languages. Compare subjects only within the same language and the same explicitly recorded language semantics, target, compile/runtime options, and dependency resolution environment. JavaScript and TypeScript MUST NOT be merged by stripping types. No cross-language duplication finding is defined by v1. Cross-language examples can instantiate the rule independently. A known different language or known different environment is a complete out-of-domain pair, not a claim that its code cannot be duplicated under another rule.

Every selected function is enumerated before pair classification. A selected unsupported function remains visible as incomplete affected scope; it MUST NOT be silently omitted or called not_applicable. An empty or below-threshold supported function is a complete nonqualifying subject. The language has a corresponding duplication construct, so missing adapter capability is incomplete, not not_applicable. Policy-excluded files remain in the declared exclusion accounting and are not secretly searched for partners. Two declarations outside the selected scope do not produce a finding within it.

Supported bodies consist exclusively of the following constructs, with complete grammar, name binding, type-contract, and operation-protocol facts as defined below:

- Ordered blocks; a bare return or return with one expression; a Rust scalar tail expression retained as its distinct return form; bare expression statements; Python pass or ECMAScript empty statements; local declarations and simple assignments to one local identifier; Rust let bindings with optional mut and a simple identifier pattern; one identifier compound assignment with a known exact language operator.
- If/else, including syntactic else-if; while; break and continue to a completely resolved lexical while target. Labels, loop-else, for, switch, match, and pattern binding are outside v1.
- Local/formal/external identifier reads; the scalar literals described in section 4; parenthesized expressions; primitive unary, binary, comparison, and short-circuit expressions with a fully identified language operation protocol. Python comparison chains are retained as chains with their evaluation semantics, not expanded into repeated binary expressions.
- Ordinary direct calls with ordered positional arguments, no spreads or keyword arguments, no dynamic/member target, and a resolved stable callable identity. A call's complete symbolic effect contract is required by section 5. Calls to either subject itself and any recursive call cycle reachable through its direct calls are outside v1; no recursion assumption is invented.
- Explicit scalar local and signature type syntax, when present, completely resolved by section 4. Function signatures have a fixed ordered list of simple identifier parameters, no default values, no variadics, no destructuring, no type parameters, and no explicit receiver. Rust borrowed scalar parameters and return types are permitted when lifetime, mutability, and ownership facts are complete.

Everything else is unsupported with a ranged reason. In particular property/subscript access, allocation/container literals, attribute mutation, reflection, eval/exec, unsafe, macros, decorators, resource acquisition, try/raise/throw/catch/finally, await/yield, and embedded foreign code are not discarded as boilerplate. Parser recovery, truncation, unread input, inconsistent snapshots, unresolved required compiler options, or unavailable source scope also produce incomplete. Language-valid unsupported source is not an error merely for being unsupported. A hard precondition failure follows the report's error contract.

These bounds are intentionally structural and independently specified. Unsupported subjects must be made complete in a later version or by an admitted extension; they do not create a true-negative exemption for the launch corpus.

## 2. Binding correspondence and canonical identities

Construct the actual lexical binding graph, including declaration kind, lexical scope, visibility of each binding at each use, definite initialization rules, shadowing, and scope-entry/exit events. A spelling match is not a binding proof. Python's function-local determination and possible unbound-local read, ECMAScript var hoisting/let-const temporal dead zone, and Rust block scoping and moves are retained. A branch block introduces a scope only where the language says it does.

Canonical traversal is the fixed node-field preorder of section 3.1: a node precedes its children; children use that table's order; list children retain source order. Signature parameters precede body traversal. Before comparison, bind every use to a declaration. Give each scope its preorder scope ordinal, and each binding within that scope its ordinal at its first binding syntax in that traversal. A Python local's first binding syntax determines its ordinal even when a read textually precedes it. Parameters receive their signature position before body-local bindings. Each local use becomes `(local, scope-ordinal, binding-ordinal, use-kind, initialization/move-state)`. Each formal use becomes `(formal, position, use-kind, initialization/move-state)`.

Permitted alpha-renaming is exactly the bijection induced by equal scope ordinals, binding ordinals, and parameter positions in equal canonical trees. It is injective and consistent over every declaration, read, and write. Names cannot be swapped by use frequency, fuzzy similarity, or searching for a convenient correspondence. Renaming MUST NOT merge shadowed bindings, change a capture, change declaration kind/mutability, or turn a local into an external. Two locals with the same spelling in different scopes remain distinct. JS var and let never normalize to the same declaration. Rust ownership moves and borrows are not erased.

Python positional-only parameter names may be alpha-renamed. Python positional-or-keyword and keyword-only parameter names are part of the signature contract and MUST retain exact spelling, because a caller can observe them. This domain excludes keyword-only parameters together with variadic/default signature forms, but the name-preservation rule is stated for extensions. ECMAScript/TypeScript and Rust simple positional parameter names are local and may be alpha-renamed. External keyword/schema/property names are never alpha-renamed. Function names may differ because the finding relates body structure; the rule does not assert that callers can substitute one declaration for another.

Every free/global/imported identifier retains an authority identity: language, resolved package/module identity including version/source authority, qualified declaration identity, and access kind. For declarations inside the analyzed project, module identity is its normalized project-relative path and qualified lexical declaration identity; for dependencies it is the pinned dependency authority and qualified exported declaration. An alias may refer to the same authority only with an actual complete alias-resolution proof. Two unrelated `normalize` functions are different authorities even with identical spelling. External mutable state, lookup timing, and rebinding semantics are retained. A missing import target or ambiguous dispatch is UNKNOWN, never a fresh local or a wildcard.

## 3. Exact normal form

The normal form is an ordered labeled tree plus its binding, type-contract, effect-protocol, and control-edge tables. Each node retains its language construct tag, semantic operator identifier, ordered children, literal/type payload when applicable, and canonical binding references. Scope and binding tables retain the descriptors from section 2. Function/signature flags, ABI/calling convention, positional/keyword slot schema, parameter mutability/ownership, and return contract are included. Source trivia and source locations are removed only from comparison and retained separately in evidence.

The only permitted syntax normalizations are removal of comments and whitespace; removal of parentheses whose grammar and evaluation role are purely grouping; canonical scalar literal spelling as section 4 specifies; and the alpha-renaming in section 2. Python pass and ECMAScript empty statements are removed after confirming they carry no labels, directives, bindings, or effects. A Python first-body docstring is removed as the explicitly permitted documentation exception; its exact source range is recorded as ignored. No other standalone string expression is removed. ECMAScript directive strings, including use-strict directives, are retained as signature/environment facts and source nodes (zero substantive weight).

Semicolons and brace/layout spelling may differ where parsing produces the same construct; grammar facts cannot be erased. Expression-body and block-body conversions, Rust tail expression versus explicit return, compound assignment versus expanded assignment, chained assignment, compare-chain expansion, else-if versus a separately scoped nested if, and declaration versus assignment are not normalized. Parentheses that change tuple formation, precedence, evaluation, or syntax meaning are retained through the resulting tree.

There is no constant folding, algebraic simplification, dead-code elimination, operand sorting, statement reordering, inlining, type erasure, common-subexpression elimination, or branch inversion. `(x + 1) + 2` differs from `x + (1 + 2)`; `a + b` differs from `b + a`; an unreachable statement still participates. This is structural equivalence rather than all programs producing the same answer.

### 3.1 Fixed node construction and comparison obligations

Use the following fixed construct tags and field order; optional absent fields use JSON null, never an empty string or an UNKNOWN token. A retained syntax distinction uses a language-qualified variant field. The variant is the grammatical construct name (for example Python augmented-assignment versus assignment, Rust return versus tail-expression); aliases invented by an adapter are prohibited. Every node payload starts with `[language, variant]` and is followed by the fields listed here in order. Child arrays appear in the stated order. Lists remain ordered and are never sorted except the explicitly unordered dependency/environment lists in section 8.

| Tag | Additional payload fields | Ordered children |
|---|---|---|
| function | function kind, signature term, environment term | body |
| block | canonical scope ID or null if no new scope | source-ordered statements |
| declare | binder ID, declaration kind, type term, mutability/ownership descriptor | initializer if present |
| assign | binder ID, assignment kind | value |
| compound-assign | binder ID, operation descriptor | right operand |
| expr-statement | none | expression |
| if | none | condition, then-block, else-block or null |
| while | loop ID | condition, body |
| break / continue | resolved loop ID | none |
| return | explicit/implicit/tail return kind | expression or null |
| read | binding reference, read kind, initialization/move descriptor | none |
| literal | literal kind, exact canonical value, type-contract term | none |
| unary | operation descriptor | operand |
| binary / short-circuit | operation descriptor | left operand, right operand |
| compare-chain | ordered operation descriptors | source-ordered operands |
| call | target authority, invocation descriptor, result-domain term | source-ordered arguments |
| directive | exact directive value | none |

Signature terms are ordered arrays of function-kind/ABI, parameter tuples in positional order, and return term. Each parameter tuple contains slot kind, canonical formal ID, externally visible name or null, type contract, and mutability/ownership descriptor. Every binding table row contains its canonical ID, owning scope, declaration kind, type contract, and initialization/ownership protocol. Scope rows contain ID, parent ID or null, and language scope kind. An operation descriptor is an ordered tuple of exact language-version operation identifier, ordered operand roles, result-domain term, and ordered effect/control protocol terms from section 5. Implicit state transitions are protocol terms, not additional weighted syntax nodes. A byte-for-byte canonical protocol identifier must denote the same published language operation; equal adapter-generated 'unknown' tags are not valid descriptors.

No unsupported construct has a tag in this table. A supported construct's absent optional syntax is a known absence, not unresolved information. Missing mandatory fields stop equality construction. The only null child placeholders are those explicitly listed in this table; they occupy their fixed child position. An absent initializer has an empty child list, while a bare return has one null child. An absent else has a null third child. Canonical node addresses are zero-based child-index paths from the function root; scope, loop, and operation table ordering uses these addresses. A control-edge row contains source address, exact edge kind, and target address or one of the distinct `function-normal-exit`, `function-exception-exit`, and `function-divergence` markers. Within each source address edge kinds are ordered by their UTF-8 bytes, then targets by their encoded bytes; this order is storage only and does not reorder execution. Language-protocol paths that cannot be represented completely yield `DUP_CONTROL_UNRESOLVED`.

The signature excludes the subject's own declaration name and source locator. All external authority identities remain. An adapter must prove its mapping from source grammar to these tags and every retained language variant. A missing mapping cannot be repaired by comparing the adapter's raw tree shape. This specification fixes the comparison operation independently of any parser's native AST field order: traverse exactly the child order in this table, and allocate binding/scope ordinals on that traversal as section 2 requires.

## 4. Literal and type contracts

Integer literals normalize to exact mathematical value together with their language numeric kind, explicit suffix, and resolved storage/operation type where the language assigns one. Base and separators may change if those fields stay equal. Unary minus remains an operator; it is not folded into a literal. ECMAScript number literals retain exact IEEE-754 binary64 value, including distinct signed zero where produced by syntax, and BigInt retains its separate kind and exact integer. Python integers remain arbitrary-precision integers. Rust integer widths, signedness, suffixes, target-dependent sizes, and overflow mode remain explicit.

Floating literals retain exact language value bits and numeric kind; approximate decimal equality is insufficient. Rust f32 differs from f64. NaN payload/sign and negative zero are not collapsed when the language/runtime can distinguish them; a payload that cannot be determined yields incomplete. String literals normalize to the exact decoded language string (Python Unicode code points; ECMAScript UTF-16 code units; Rust Unicode scalar string), without Unicode normalization. Byte strings remain a separate exact byte sequence and language kind. Bool, Python None, ECMAScript null, and language unit are distinct tags. Interpolated/formatted strings, regex, raw pointers, and unknown literal kinds are outside v1. A syntactic identifier such as JS undefined is an identifier requiring resolution, not a magic literal.

Type contracts are ordered resolved terms. Primitive types retain language, kind, width, signedness, nullability, reference/ownership and mutability descriptors. Nominal types retain their exact authority; aliases retain alias authority and resolved target (no alias erasure). Union/intersection member order and explicit syntax distinctions are preserved; no equivalence by assignability, subtype, or approximate annotation name is permitted. TypeScript any and unknown are explicit distinct contracts, never missing-fact placeholders. TypeScript inferred types and Rust inferred scalar types require complete recorded inference. Erased TypeScript assertions, non-null assertions, and overload/generic forms are unsupported rather than converted to JavaScript.

Python absence of annotation and JavaScript absence of a static type system are known contract facts, represented respectively as `python-runtime-domain` and `ecmascript-runtime-domain` at each formal slot. These denote the language's actual runtime domain, not an analyzer's unresolved type. A Python annotation is preserved as a resolved annotation term and does not falsely imply enforcement of runtime operand types. Dynamic result domains may be symbolic terms indexed by canonical inputs and exact operation protocols. Such a symbolic term is complete only when all of its dependencies and protocol choices are known. An absent Python annotation is not an unresolved annotation; an unread annotation or missing Rust/TypeScript inference is unresolved.

Consequently, two unannotated Python bodies can be equivalent by the same exact runtime protocols on corresponding formal values, without claiming that their values are integers. A dynamic domain MUST NOT be used to replace an unresolved imported type, lost annotation, unavailable overload choice, or incomplete operand-origin analysis. Type differences that are fully known refute equivalence; missing facts do not.

## 5. Effects, order, and control

Preserve the ordered evaluation tree and all control transfers: branch condition/true/false targets; loop condition/body/backedge/exit; each break/continue target; return and implicit-return/tail rules; and normal, exceptional, initialization-failure, move-failure, or divergence possibilities prescribed by each operation. Explicit exceptional/resource syntax is outside v1, but the possibility that supported arithmetic or a call raises, traps, overflows, or diverges is never erased.

An operation's complete effect descriptor identifies the exact language protocol and its operand roles, result-domain term, external read/write authority if any, and possible normal/exceptional/divergent transitions. For Python runtime addition, for example, it preserves the exact language-version protocol for operand/reflected dispatch and possible coercion/error, indexed by the ordered canonical operand terms. This is not a claim of purity. ECMAScript conversions are retained similarly. Rust primitive arithmetic includes the resolved type and overflow behavior; user-defined trait dispatch needs exact resolved target and contract or is incomplete.

A direct call retains the exact resolved stable target authority, ordered argument terms, calling convention, result contract, and effect expression. Its effect expression may be a complete symbolic invocation of the same authority on corresponding arguments; that compares an identical action, without requiring the analyzer to enumerate every transitive internal effect. It requires proof of binding/lookup stability and a known invocation protocol. It MUST NOT be replaced by 'pure', 'unknown effect', or 'some call'. If rebinding, runtime lookup, dispatch, or call shape is unresolved, the call is incomplete. A caller's instruction order remains significant even when two callees are proved pure.

These protocol descriptors are semantic obligations, not permission to trust identical syntax despite missing facts. A descriptor requires an identified supported language operation and complete operand-binding terms. Known arbitrary runtime operand values are universally quantified over corresponding formal slots. Unknown operation identity is an epistemic hole and cannot be universally quantified into equality. Effects derived from distinct external authorities differ even when they have the same broad read/write categories.

Two trees with different evaluation order, branch polarity, loop target, short-circuit mode, mutation destination, ownership behavior, cleanup behavior, possible return form, or effect descriptor are not equivalent. A symbolic effect equation can compare equal only by exact identical protocol/authority and identical canonical dependency terms; no theorem proving or guessed effect compatibility is admitted.

## 6. Minimum substantive structure and boilerplate

Compute substantive weight W on the normalized body only. Every unary operation, binary operation, comparison operator (each comparison link in a chain), and short-circuit connective contributes 1. Each if and while contributes 1. Count an identifier compound assignment once for its operator. No other node contributes: declarations, stores, identifier/literal leaves, blocks, parameters, calls, returns, break/continue, annotations, comments, directives, or documentation contribute 0. Nested operations are each counted once. No expanded or implicit operation is counted.

Eligibility requires W >= 8. It also requires at least one of:

(a) an if or while and at least three unary/binary/comparison/short-circuit operation nodes; or
(b) a chain of at least three operation nodes linked by immediate parent-child operand edges within one expression.

For (b), an operand edge directly joins operation nodes; an intervening local variable use, call, store, or statement boundary breaks the chain. For (a), operations may occur anywhere in the body, including branch/loop conditions. These two alternatives and W are evaluated on actual normalized syntax, not execution frequency. There is no line-count, token-count, similarity percentage, shared-name count, or shared-callee score.

Eight is a normative exact threshold defining v1's reportable granularity, chosen here without detector calibration. W=7 does not qualify; W=8 still requires (a) or (b). An extra return, assignment, or shared utility call cannot inflate W. Repetition of eight disconnected `helper(value)` calls has W=0 and proves nothing. Shared utility targets can appear inside an otherwise qualifying equivalent body, but their identities only constrain equivalence; they never establish substantive structure.

Comments, formatting, empty statements, and the exact documentation exception in section 3 are the only ignored boilerplate. Imports are external binding prerequisites rather than body decoration. Public APIs, tests, generated code, registration functions, wrapper names, logging, validation, and framework glue have no blanket exemption. If supported, they obey the same rule; if unsupported, they are incomplete. Counting arithmetic inside an unsupported node cannot salvage eligibility because the whole subject remains unsupported.

## 7. Pair decision, findings, and incompleteness

For every unordered pair of enumerated distinct subjects, apply these rules in order:

1. A known disallowed subject-kind, selected unsupported construct, or missing required fact produces INCOMPLETE for that subject and its affected comparisons, with exact reason and range. However, a fully known independent disqualifier may establish a nonqualifying pair: a known different language/environment, or either fully counted W < 8, or a completely known failure of both shape alternatives. Unknown weight is not low weight. An independent pair disqualifier does not erase the incomplete subject's required report accounting.
2. For eligible supported subjects with complete facts, compare their entire normal forms and all tables exactly. Equality produces EQUIVALENT; a known differing field produces DISTINCT. An unresolved field needed to decide equality produces INCOMPLETE, not DISTINCT. A known differing field can refute a pair even if another field is unresolved, but the unresolved subject remains incomplete in report scope.
3. Emit exactly one duplication finding per EQUIVALENT unordered pair. For three equal subjects emit three pairs, not a hidden representative-only group. A grouping presentation MAY reference all pair IDs; it cannot change full counts. No confidence score or filter can convert INCOMPLETE to DISTINCT or clean.

Supported complete nonqualifying/distinct pairs are ruled negatives only under v1. They are not claims of general program inequivalence. Incomplete comparisons never qualify as true negatives. Positive pairs with complete proofs remain valid in a larger incomplete run. The duplication pattern is complete only when enumeration is complete and every in-scope subject/comparison obligation is complete; positive findings and the overall incomplete outcome can coexist. Publication of this rule does not admit an incomplete binding acceptance case.

Use stable reason codes with ranges and dependencies: `DUP_UNSUPPORTED_SUBJECT`, `DUP_UNSUPPORTED_CONSTRUCT`, `DUP_PARSE_LOSS`, `DUP_BINDING_UNRESOLVED`, `DUP_TYPE_UNRESOLVED`, `DUP_EFFECT_UNRESOLVED`, `DUP_CONTROL_UNRESOLVED`, `DUP_ENVIRONMENT_UNRESOLVED`, `DUP_INPUT_LOSS`, and `DUP_SCOPE_UNENUMERATED`. An independent threshold refutation is `DUP_BELOW_THRESHOLD`; known shape failure is `DUP_SHAPE_THRESHOLD`; known out-of-domain language/environment is `DUP_DIFFERENT_DOMAIN`; a complete field mismatch is `DUP_DISTINCT_FIELD`. Reasons MUST identify the concrete missing/differing field; repeating a generic unknown over every file is insufficient. Missing facts cannot become invented equivalence or universal suppression of already provable pairs.

## 8. Evidence identity and deterministic bytes

A finding's semantic subject is the unordered pair of function authorities within a declared scope; its relation is `binding-aware-whole-body-equivalence`. Its structured evidence includes rule version and SHA-256 of this exact public document; language/environment identity; both function/signature and body half-open UTF-8 byte ranges; both input hashes; exact normalized tree and tables or content-addressed accessible copies; the complete binder bijection; ignored-range list; W and the qualifying shape witness; full external/type/effect dependency proofs; and the common normal-form digest. Every ignored range is distinguished from missing input. Evidence cannot be just a similarity score or common-callee list.

Define a function locator as `(language, normalized-project-relative-path, ordered-lexical-declaration-name-path, same-name-declaration-ordinal)`. Paths use forward slashes, no dot components, and preserve case; lexical declaration names preserve exact Unicode code points without Unicode normalization. The ordinal is zero-based among same-name declarations in that lexical owner in source order. Body-local renames and formatting do not alter this locator. Moving or renaming a function may change it; continuity must not be invented from line proximity.

Sort the two locator tuples lexicographically by their UTF-8 serialized bytes. Encode `['duplication', 'duplication-normalization-v1', 'binding-aware-whole-body-equivalence', locatorA, locatorB]` as compact JSON arrays with no whitespace, UTF-8 non-ASCII characters unescaped, escaping only JSON-required quotes, backslashes, and U+0000..U+001F (lowercase four-digit hex for control characters; no short control escapes). The stable finding ID is `duplication:` followed by lowercase SHA-256 hex of these bytes. Ranges, input hashes, timestamps, and normal-form digests are evidence, not locator fields. Sorting output by this ID yields deterministic pagination independent of parallel traversal.

Normal forms serialize as ordered arrays, never unordered maps: node `[tag, payload-array, children-array]`, with canonical scope/binding/type/effect/control tables appended in the established preorder/index order. Payload strings follow the same JSON escaping rule. Integers use base-10 JSON integer spelling without leading zeros; floating values use fixed-width lowercase hex bit strings, never JSON floats. Dependency authority tuples and environment options are sorted by their UTF-8 encoding before serialization. Domain and table schema tags are included. A SHA-256 match is an index; equivalence still requires equality of canonical bytes and complete proofs, so a hash collision cannot prove a finding. Any future change to normalization, threshold, serialization, domain, or identity requires a new version and independent admission.

## 9. Rule-derived examples and adversarial rivals

These examples assume fully read/parsed scopes and the language/runtime facts stated here. They are explanatory rule witnesses, not source-oracle admission or measured product results.

### Positive P1: alpha-renamed Python positional-only bodies

```python
def adjust_a(value, /):
    t = ((value + 3) * 2) - 1
    if (t > 10) and (value != 0):
        return (t + 4) * 5
    return t - 2

def adjust_b(item, /):
    z = ((item + 3) * 2) - 1
    if (z > 10) and (item != 0):
        return (z + 4) * 5
    return z - 2
```

W=10: three initial arithmetic operators; two comparisons and one short-circuit connective; two then-return arithmetic operators; one final subtraction; one if. Alternative (a) holds and the initial expression also supplies a three-operation chain for (b). The binding bijection is formal position 0 to 0, local 0 to 0. Neither requires an invented integer type or purity: Python runtime arithmetic/comparison/short-circuit protocols operate on corresponding symbolic operand values in the same order. A missing protocol or binding fact would instead be incomplete. Whitespace, comments, and harmless grouping changes are false-refusal rivals: they MUST preserve this positive.

### Positive P2: exact same rule in the other three languages

Use two declarations of the following body, changing only function name, the positional parameter name, and the local name consistently. JavaScript uses ordinary function declarations, TypeScript uses a `number` parameter and return annotation, and Rust uses `i64` parameter/return with `let` local. Environment and arithmetic semantics are identical within each pair. This is three separate positive pairs, never a cross-language finding.

```javascript
function adjustA(value) {
  const t = ((value + 3) * 2) - 1;
  if ((t > 10) && (value !== 0)) { return (t + 4) * 5; }
  return t - 2;
}
```

```typescript
function adjustA(value: number): number {
  const t = ((value + 3) * 2) - 1;
  if ((t > 10) && (value !== 0)) { return (t + 4) * 5; }
  return t - 2;
}
```

```rust
fn adjust_a(value: i64) -> i64 {
    let t = ((value + 3) * 2) - 1;
    if (t > 10) && (value != 0) { return (t + 4) * 5; }
    return t - 2;
}
```

Each has W=10 and meets (a) and (b). JS may be dynamically typed without epistemically missing operand origins. TS inferred local type must be available. Rust overflow mode and i64 behavior must be pinned. Refusing all unannotated JS because it lacks static types, all Rust because arithmetic might overflow, or all TS because annotations erase at runtime would be false refusals of these fully specified pairs.

### Positive P3: exact threshold boundary

```python
def boundary_a(x, /):
    t = ((x + 1) * 2) - 3
    if t > 0:
        return (t + 4) * 5
    return t - 6
```

Its alpha-renamed copy is equivalent: W=8 (3 + comparison 1 + if 1 + then 2 + final 1), meeting both alternatives. Replacing the then expression with `t + 4` gives W=7; two copies of that smaller body are complete nonqualifying subjects. Additional calls/returns/assignments cannot restore eligibility. An implementation using `W > 8` or counting only lines would falsely refuse P3.

### Near misses and completeness rivals

| Rival derived from a positive or threshold rule | Required disposition and reason |
|---|---|
| P1 second body's literal 3 becomes 4 | DISTINCT: literal value differs; names and CFG similarity cannot compensate. |
| P1 second body's `z + 4` becomes `4 + z` | DISTINCT: operand order differs, including dynamic protocol dispatch. |
| P1 loses `/` and its public keyword parameter names differ | DISTINCT: keyword-slot schema differs. Consistent body renaming does not erase named-call contracts. |
| Equal P1 bodies with one extra supported local assignment | DISTINCT: stores have zero threshold weight but remain structural nodes. |
| One body uses the outer local in its condition, the other a different/shadowed local | DISTINCT when binding is proved; INCOMPLETE when resolution is missing. Spelling does not repair the mismatch. |
| JS body changes `const` to `let`, `!==` to `!=`, or `&&` to `&` | DISTINCT: declaration/operation protocol differs, even if observed inputs happen to agree. |
| Rust pair changes one parameter/return from i64 to u64 | DISTINCT: exact types and arithmetic domains differ. |
| TS pair changes number to any | DISTINCT: explicit type-contract terms differ. Missing inferred type instead means INCOMPLETE. |
| Two bodies retain equivalent numeric base spelling or escape spelling for the same exact string kind/value | Same equivalence disposition as before; refusing only for spelling is a false refusal. |
| Two equal-looking bodies call differently resolved authorities with the same callee spelling | DISTINCT: external identity differs. If one target is unresolved, INCOMPLETE. |
| Eight separate supported `x + 1` expression statements, no control node and no nested operation chain | Nonqualifying: W=8 but neither (a) nor (b) holds. Weight alone is insufficient. |
| One return expression with eight nested binary operations, including a three-operation immediate operand chain | Its exact alpha-renamed copy qualifies by (b), even without if/while. A control-only implementation would falsely refuse it. |
| A while pair differs only in a resolved break/continue target or condition evaluation | DISTINCT: control edge targets/protocol differ; a missing target is INCOMPLETE. |
| Equal Python bodies differ only in first-body docstring text, or in a pass statement | Same equivalence disposition: explicit documented boilerplate exception. A later string expression remains a real structural node. |
| An ECMAScript use-strict directive appears in only one body | DISTINCT when semantics are fully known: directive/environment fact retained. |
| Two copies only call `helper(x)` repeatedly and return a call | Nonqualifying: W=0. A shared utility call list is insufficient. |
| Two otherwise qualifying bodies have identical proved stable calls at corresponding positions | EQUIVALENT if every other fact matches; calls add zero W. Blanket refusal of all effects would be false refusal. |
| Equal-looking bodies differ in if condition polarity or mutation target | DISTINCT: exact control/binding terms differ. No branch inversion or destination guessing. |
| Equal-looking bodies contain try/finally, reflection, decorators, methods, or a dynamic member call | INCOMPLETE with exact unsupported range; no stripping or declaration of clean. |
| One parser recovers through a missing branch | INCOMPLETE: missing branch is not normalized away. |
| A separate selected unsupported function sits beside the complete P1 pair | P1 finding retained; overall duplication scope incomplete. Global silence is a false refusal. |
| Read race changes one body after its input hash | INCOMPLETE; never compare a mixed snapshot or claim resolution in diff. |

Any negative in this table concerns the exact stated pair, not every possible pair in the surrounding file. Real source-case authors must establish all relevant source facts independently before assigning expected/forbidden findings. A distinct rule judge must admit or return specific REWORK for these exact public bytes and version; neither authorship nor any example qualifies the product or the release.
