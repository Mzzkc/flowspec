# Language and diagnostic registration

`flowspec registry` prints the actual default catalog and its digest as JSON.
`flowspec::registry::Registry::default()` supplies the same descriptors to the
library. The four default IDs are python, javascript, typescript and rust.
Their extensions are py; js/jsx/mjs/cjs; ts/tsx; and rs respectively. Canonical
catalog order matches the thirteen pattern IDs in the analysis report's scope;
legacy detector invocation order is preserved separately.

The instance-local public extension seam is `Registry::register_language` and
`analyze_with_registry(path, config, languages, &registry)`. Supply an owned
`LanguageDescriptor` with unique ID, aliases and extensions, stable implementation,
adapter and grammar version identifiers, explicit capabilities and limitations,
and a factory returning `Box<dyn LanguageAdapter>`. Registrations are validated
atomically. A custom language remains disabled by default; select its ID or alias
explicitly. Missing recovery observation and missing diagnostic capabilities enter
the ordinary report as information loss. Registration does not qualify a fifth
language or any diagnostic's semantic completeness.

`Registry::metadata()` and the CLI catalog contain the full descriptors. The
canonical digest covers language identity, aliases, extensions, factory/recovery
implementation identifiers, versions, capabilities, limitations, default state,
and each diagnostic's identity, detector identifier/version, requirements,
applicability and invocation order. It excludes callback addresses. A callback's
implementation identifier/version must change when its behavior changes.

The bounded analysis report preserves its accepted provenance registry digest
field. That digest binds the complete descriptor metadata exposed through the
registry API and catalog command. The report already carries the full ordered
catalog in `scope.pattern_ids`; selected adapters retain adapter/grammar versions
and capability digests, and per-pattern requirements remain in pattern state.
Repeating full registry descriptors in every small analysis report would violate
the preserved output budgets. SARIF consumes the ordered catalog from the same
report and expands it into canonical rule IDs with visible incomplete execution.

TypeScript still uses the JavaScript grammar with preprocessing. Native TypeScript,
structured AdapterOutput, cache conformance, full type flow, independent gold
labels, strict Clippy, coverage and release qualification remain separate work.

Production module resolution uses language/package/module identity and the actual
importing binding. Python and Rust cannot bind an unrelated same-name language.
JS/TS relative imports may interoperate via actual source paths; extensionless
same-stem rivals remain unresolved rather than selecting by map order. Package
anchors come from the nearest language manifest within the declared project root.
The public legacy `build_module_map` is a compatibility view, not the production
namespace authority; collisions retain path-qualified entries while ambiguous
unqualified keys are omitted.
