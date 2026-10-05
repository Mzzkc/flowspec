# One analysis/change loop

Resolve `FS` to the installed archive's `flowspec/bin/flowspec`. Choose `PROJECT`
explicitly and preserve the reports outside that analyzed source tree.

```sh
"$FS" --version
"$FS" analyze "$PROJECT" --gate-severity info --gate-confidence low --format json --output before.json
"$FS" diagnose "$PROJECT" --gate-severity info --gate-confidence low --format summary
"$FS" trace "$PROJECT" --symbol 'main.py::main' --format json
# Apply the smallest change and run the repository's documented behavior tests.
"$FS" analyze "$PROJECT" --gate-severity info --gate-confidence low --format json --output after.json
"$FS" diff before.json after.json --format json
# Run the repository's complete native validation command, then the structural gate.
"$FS" diagnose "$PROJECT" --gate-severity info --gate-confidence low --format summary
```

Capture each exit code even when shell execution continues. Read the JSON
`authority` before interpreting absence or resolution. Check trace paths against
actual calls; do not rely on a subsequence or partial symbol search.

Use `--language` only when the intended claim is restricted to that language.
Record patterns, paths, exclusions and thresholds as scope. Preserve full counts
when applying severity or confidence display filters. Filters are not permission
to suppress incompleteness.

Run `analyze --help` before using pagination. A candidate that rejects `--limit`
or `--offset` has not demonstrated CLI continuation. Store the unfiltered report,
record HOLD, and avoid manufacturing continuation tokens. If exact finding-ID
selection is unavailable, inspect the full canonical finding catalog by stable
ID; keep command-selector qualification held.

If `diff` returns 3 for complete same-scope before/after inputs, preserve that
output as an implementation limitation. Do not edit identities, remove losses,
or regenerate reports through a different binary to force comparability.

Native validation must come from the repository's contributor instructions.
An empty analysis fixture is useful for exit plumbing, not evidence that a real
repository meets the thirteen-diagnostic launch oracle.
