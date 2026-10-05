---
name: flowspec
description: This skill should be used when the user asks to "run flowspec", "analyze code structure", "diagnose structural findings", "trace a symbol", or "check changes with flowspec diff".
---

# FLOWSpec

Treat analysis as evidence for the declared scope. Check completeness before interpreting absence. A development archive does not certify a release or the diagnostic semantics.

1. **Analyze a bounded scope.** Confirm the packaged binary with `bin/flowspec --version` and inspect `--help`. Run `bin/flowspec analyze PROJECT --format summary` for orientation; save JSON with the same scope, config, languages, and gate thresholds before editing. Restrict scope deliberately and record exclusions. Never call a restricted result a whole-repository result.
2. **Check authority.** Read outcome, execution, inventory, losses, per-pattern state, full finding counts, and continuation from the report's `authority` object. Trust absence only for complete declared scope. Treat a visibly justified `not_applicable` cell as outside that diagnostic's applicable scope, not a proved negative.
3. **Inspect exact evidence.** Run `diagnose PROJECT --format json` with matching gate thresholds. Use stable finding IDs and source ranges to locate findings; inspect structured evidence before editing. Run `trace PROJECT --symbol EXACT_ID --format json` using the exact entity ID from analysis, for example `main.py::main`. Read the entire ordered edge path. If a finding-ID selector or exact trace operation is unsupported, record HOLD and inspect available evidence; never invent a command or promote an approximate match.
4. **Make the smallest tested change.** Follow the repository's test policy and validate the affected behavior. Preserve the original report and native test evidence.
5. **Analyze and diff.** Save a new JSON report using identical declared scope and thresholds. Run `diff before.json after.json --format json`. Require complete, comparable inputs before asserting resolution or no regression. An incomplete or incomparable diff blocks that assertion.
6. **Validate the repository.** Run its own documented checks plus `diagnose PROJECT --gate-severity info --gate-confidence low --format summary`. Record both outcomes. FLOWSpec's structural gate cannot replace native validation.

| Exit | Meaning | Action |
| --- | --- | --- |
| 0 | Complete; selected gate passes | Check full counts and scope before a clean claim |
| 1 | Execution error | Repair the precondition; no contract-valid clean result |
| 2 | Complete; usable findings meet the gate | Inspect evidence; exit 2 is not tool failure |
| 3 | Incomplete, with valid findings retained | Exit 3 blocks clean and regression claims |

Outcome and gate are separate: a complete report can contain findings below the selected gate and exit 0. Read full counts before calling it clean. Display filters never authorize discarding outcome or completeness.

Check `authority.continuation`: `full`, `matched_total`, `shown`, `truncated`, and `next_cursor`. Follow a supported cursor until the selected details are exhausted. If CLI limit/offset or cursor continuation is unavailable, retain the full JSON artifact and record HOLD for pagination; do not pipe away report authority or call a truncated view complete. Summary is an orientation view, not a substitute for exact evidence.

Load [report-contract](references/report-contract.md) for schema, comparison, and trust details. Load [examples](references/examples.md) for the save/trace/diff loop and unsupported-operation handling. Never interpret an empty incomplete report as clean. Do not change acceptance expectations to make a candidate pass.
