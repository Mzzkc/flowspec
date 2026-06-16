#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later AND LicenseRef-Commercial
"""validate-oracle-artifacts.py - semantic oracle-contract gate.

This validator parses every oracle artifact, runs the comparator smoke, and
executes live fixture/provenance checks where the release gate would otherwise
be gameable. It exits 0 only when every non-skipped check is green.
"""

from __future__ import annotations

import argparse
import glob
import hashlib
import json
import os
import subprocess
import sys
from collections import Counter, defaultdict
from typing import Any

import yaml

CANONICAL_PATTERNS = [
    "isolated_cluster",
    "data_dead_end",
    "partial_wiring",
    "orphaned_impl",
    "duplication",
    "contract_mismatch",
    "circular_dependency",
    "layer_violation",
    "incomplete_migration",
    "asymmetric_handling",
    "stale_reference",
    "phantom_dependency",
    "missing_reexport",
]
CANONICAL_SET = frozenset(CANONICAL_PATTERNS)

CLASSIFICATIONS = frozenset({"REAL", "KNOWN_FP", "DEFERRED_BOUNDARY"})
REQUIRED_TARGETS = ("self", "marianne")
REQUIRED_TARGET_SET = frozenset(REQUIRED_TARGETS)

CLASSIFICATION_FIELDS = [
    "pattern",
    "path",
    "line",
    "symbol",
    "classification",
    "reason",
    "owner",
    "expires",
]

REQUIRED_RULE_IDS = [
    "R001_UNCLASSIFIED_FINDING",
    "R002_PRECISION_BELOW_THRESHOLD",
    "R003_REAL_DISAPPEARED_UNREVIEWED",
    "R004_KNOWN_FP_STILL_HIGH_CONF",
    "R005_TRACE_MISSING_PATH",
    "R006_SCRATCH_IN_DIAGNOSTICS",
    "R007_FULL_VS_INCREMENTAL_MISMATCH",
    "R008_COUNT_DELTA_UNREVIEWED",
]

SEMANTIC_SMOKE_RULE_IDS = (
    "R001_UNCLASSIFIED_FINDING",
    "R002_PRECISION_BELOW_THRESHOLD",
    "R003_REAL_DISAPPEARED_UNREVIEWED",
    "R004_KNOWN_FP_STILL_HIGH_CONF",
    "R005_TRACE_MISSING_PATH",
    "R006_SCRATCH_IN_DIAGNOSTICS",
    "R008_COUNT_DELTA_UNREVIEWED",
)

VALID_COVERAGE_STATUS = frozenset({"implemented", "deferred", "off-by-default"})
PROVENANCE_TOP_KEYS = ["git", "binary", "version", "commands", "targets"]
PRECISION_THRESHOLD = 0.80
FALLBACK_FLOWSPEC_BIN = None  # resolved repo-relative in resolve_binary() (no hardcoded host paths)
TBD = "TBD-FILL-IN-1B"

EXPECTED_SELFTEST_FAILURE_MARKERS = {
    "BAD": [
        ("duplicate-collapse classification", "count-equiv", "duplicate-collapse"),
        ("strict positive fixture truth", "fixture-coverage", "positive_fixture did NOT fire"),
    ],
    "PROD": [
        ("required target coverage", "required-target-coverage", "REQUIRED_TARGET_MISSING"),
        ("production non-live raw_source", "count-equiv", "non-live raw_source rejected"),
        ("production verified:false", "antifab", "verified:false is forbidden"),
        ("post-baseline TBD provenance", "provenance", TBD),
    ],
}

PASS = "PASS"
FAIL = "FAIL"
SKIP = "SKIP"


class Report:
    def __init__(self, label: str) -> None:
        self.label = label
        self.results: list[tuple[str, str, str]] = []

    def add(self, check: str, status: str, detail: str) -> None:
        self.results.append((check, status, detail))

    @property
    def failed(self) -> bool:
        return any(s == FAIL for _, s, _ in self.results)

    def summary_line(self) -> str:
        n_pass = sum(1 for _, s, _ in self.results if s == PASS)
        n_fail = sum(1 for _, s, _ in self.results if s == FAIL)
        n_skip = sum(1 for _, s, _ in self.results if s == SKIP)
        return f"{self.label}: {n_pass} pass, {n_fail} FAIL, {n_skip} skip"

    def print(self) -> None:
        print(f"\n=== {self.label} ===")
        for check, status, detail in self.results:
            tag = {"PASS": "ok  ", "FAIL": "FAIL", "SKIP": "skip"}[status]
            print(f"  [{tag}] {check}: {detail}")


def load_yaml(path: str) -> Any:
    with open(path, "r", encoding="utf-8") as fh:
        return yaml.safe_load(fh)


def repo_root_from_script() -> str:
    return os.path.dirname(os.path.dirname(os.path.abspath(__file__)))


def resolve_binary(path: str) -> str:
    if os.path.isfile(path):
        return path
    if not os.path.isabs(path):
        local = os.path.join(repo_root_from_script(), path)
        if os.path.isfile(local):
            return local
    fallback = os.path.join(repo_root_from_script(), "target", "release", "flowspec")
    if os.path.isfile(fallback):
        return fallback
    return path


def parse_loc(loc: str) -> tuple[str, int] | None:
    if not isinstance(loc, str) or ":" not in loc:
        return None
    path, _, line_str = loc.rpartition(":")
    try:
        return path.strip(), int(line_str.strip())
    except ValueError:
        return None


def file_line_count(path: str) -> int | None:
    try:
        with open(path, "r", encoding="utf-8", errors="replace") as fh:
            return sum(1 for _ in fh)
    except OSError:
        return None


def is_selftest_path(path: str) -> bool:
    norm = os.path.abspath(path).replace(os.sep, "/")
    return "/tests/oracle/_selftest/" in f"{norm}/"


def has_real_diagnostics(oracle_dir: str) -> bool:
    return (not is_selftest_path(oracle_dir)) and bool(
        glob.glob(os.path.join(oracle_dir, "diagnostics", "*.yaml"))
    )


def resolve_target_root(target_root: str, oracle_dir: str) -> str:
    if os.path.isabs(target_root):
        return os.path.normpath(target_root)
    return os.path.normpath(os.path.join(oracle_dir, target_root))


def resolve_artifact_path(value: str, oracle_dir: str) -> str:
    if os.path.isabs(value):
        return os.path.normpath(value)
    root_candidate = os.path.join(repo_root_from_script(), value)
    if os.path.exists(root_candidate):
        return os.path.normpath(root_candidate)
    return os.path.normpath(os.path.join(oracle_dir, value))


def finding_symbol(finding: dict[str, Any]) -> str:
    return str(finding.get("symbol", finding.get("entity", "")))


def finding_key(finding: dict[str, Any]) -> tuple[str, str, int, str] | None:
    parsed = parse_loc(str(finding.get("loc", "")))
    pat = finding.get("pattern")
    if parsed and pat:
        return (str(pat), parsed[0], parsed[1], finding_symbol(finding))
    return None


def entry_key(entry: dict[str, Any]) -> tuple[str, str, int, str] | None:
    pat = entry.get("pattern")
    ep = entry.get("path")
    el = entry.get("line")
    symbol = entry.get("symbol")
    if pat and isinstance(ep, str) and isinstance(el, int) and symbol:
        return (str(pat), ep, el, str(symbol))
    return None


def optional_fingerprint_mismatches(entry: dict[str, Any], finding: dict[str, Any]) -> list[str]:
    pairs = (
        ("severity_at_review", "severity"),
        ("severity", "severity"),
        ("confidence_at_review", "confidence"),
        ("confidence", "confidence"),
        ("message_at_review", "message"),
        ("message", "message"),
        ("range_at_review", "range"),
        ("range", "range"),
    )
    bad = []
    for entry_field, raw_field in pairs:
        if entry_field in entry and entry.get(entry_field) not in (None, ""):
            if entry.get(entry_field) != finding.get(raw_field):
                bad.append(f"{entry_field}={entry.get(entry_field)!r} raw {raw_field}={finding.get(raw_field)!r}")
    return bad


def counter_examples(counter: Counter[tuple[str, str, int, str]], limit: int = 3) -> list[Any]:
    out: list[Any] = []
    for key, count in sorted(counter.items()):
        out.append({"key": key, "count": count})
        if len(out) >= limit:
            break
    return out


def load_classification_files(oracle_dir: str) -> list[tuple[str, dict[str, Any]]]:
    out = []
    for path in sorted(glob.glob(os.path.join(oracle_dir, "diagnostics", "*.yaml"))):
        try:
            data = load_yaml(path)
        except yaml.YAMLError as exc:
            out.append((path, {"__parse_error__": str(exc)}))
            continue
        if isinstance(data, dict):
            out.append((path, data))
    return out


def check_schema(report: Report, files: list[tuple[str, dict[str, Any]]]) -> None:
    if not files:
        report.add("schema", SKIP, "no diagnostics/*.yaml yet (baseline not populated)")
        return
    total = 0
    bad = 0
    for path, data in files:
        if "__parse_error__" in data:
            report.add("schema", FAIL, f"{os.path.basename(path)}: unparseable YAML: {data['__parse_error__']}")
            bad += 1
            continue
        entries = data.get("entries")
        if not isinstance(entries, list):
            report.add("schema", FAIL, f"{os.path.basename(path)}: missing or non-list 'entries'")
            bad += 1
            continue
        for i, entry in enumerate(entries):
            total += 1
            if not isinstance(entry, dict):
                report.add("schema", FAIL, f"{os.path.basename(path)} entry#{i}: not a mapping")
                bad += 1
                continue
            for field in CLASSIFICATION_FIELDS:
                if field not in entry or entry[field] in (None, ""):
                    report.add(
                        "schema",
                        FAIL,
                        f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                        f"missing/empty required field '{field}'",
                    )
                    bad += 1
            if entry.get("classification") not in CLASSIFICATIONS:
                report.add(
                    "schema",
                    FAIL,
                    f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                    f"classification '{entry.get('classification')}' not in {sorted(CLASSIFICATIONS)}",
                )
                bad += 1
            if entry.get("pattern") not in CANONICAL_SET:
                report.add(
                    "schema",
                    FAIL,
                    f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                    f"pattern '{entry.get('pattern')}' not one of the 13 canonical IDs",
                )
                bad += 1
            line = entry.get("line")
            if not isinstance(line, int) or isinstance(line, bool) or line < 1:
                report.add(
                    "schema",
                    FAIL,
                    f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                    f"line '{line}' is not a positive int",
                )
                bad += 1
    if bad == 0:
        report.add("schema", PASS, f"{total} classification entries across {len(files)} file(s) valid")
    else:
        report.add("schema", FAIL, f"{bad} schema violation(s) across {len(files)} file(s)")


def check_antifabrication(report: Report, files: list[tuple[str, dict[str, Any]]], oracle_dir: str) -> None:
    if not files:
        report.add("antifab", SKIP, "no diagnostics/*.yaml yet (baseline not populated)")
        return
    checked = 0
    bad = 0
    production = not is_selftest_path(oracle_dir)
    for path, data in files:
        if "__parse_error__" in data:
            continue
        entries = data.get("entries")
        if not isinstance(entries, list):
            continue
        target_root = resolve_target_root(str(data.get("target_root", ".")), oracle_dir)
        for i, entry in enumerate(entries):
            if not isinstance(entry, dict):
                continue
            if entry.get("verified") is False:
                if production:
                    report.add(
                        "antifab",
                        FAIL,
                        f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                        "verified:false is forbidden outside tests/oracle/_selftest",
                    )
                    bad += 1
                    continue
                if not entry.get("verify_skip_reason"):
                    report.add(
                        "antifab",
                        FAIL,
                        f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                        "verified:false requires a non-empty verify_skip_reason",
                    )
                    bad += 1
                continue
            ep = entry.get("path")
            el = entry.get("line")
            if not isinstance(ep, str) or not isinstance(el, int):
                continue
            full = os.path.join(target_root, ep)
            if not os.path.isfile(full):
                report.add(
                    "antifab",
                    FAIL,
                    f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                    f"cited path does not exist on disk: {full}",
                )
                bad += 1
                continue
            n = file_line_count(full)
            if n is None or el < 1 or el > n:
                report.add(
                    "antifab",
                    FAIL,
                    f"{os.path.basename(path)} entry#{i} ({entry.get('symbol','?')}): "
                    f"line {el} out of range [1,{n}] for {ep}",
                )
                bad += 1
                continue
            checked += 1
    if bad == 0:
        report.add("antifab", PASS, f"{checked} cited path:line(s) verified to exist on disk")
    else:
        report.add("antifab", FAIL, f"{bad} fabricated/out-of-range path:line citation(s)")


def check_required_target_coverage(report: Report, files: list[tuple[str, dict[str, Any]]], oracle_dir: str) -> None:
    if not files:
        report.add("required-target-coverage", SKIP, "no diagnostics/*.yaml yet (baseline not populated)")
        return
    if is_selftest_path(oracle_dir):
        report.add("required-target-coverage", SKIP, "synthetic _selftest samples may use planted targets")
        return

    bad = 0
    seen: set[str] = set()
    entry_counts: dict[str, int] = {}
    target_roots: dict[str, str] = {}
    expected_files = [f"{target}.yaml" for target in REQUIRED_TARGETS]

    for path, data in files:
        name = os.path.basename(path)
        stem, _ = os.path.splitext(name)
        if "__parse_error__" in data:
            report.add("required-target-coverage", FAIL, f"REQUIRED_TARGET_PARSE_ERROR: {name} is unparseable")
            bad += 1
            continue

        declared = data.get("target")
        if stem not in REQUIRED_TARGET_SET:
            report.add(
                "required-target-coverage",
                FAIL,
                f"REQUIRED_TARGET_UNKNOWN: {name} is not a declared target file; expected {expected_files}",
            )
            bad += 1
        if declared not in REQUIRED_TARGET_SET:
            report.add(
                "required-target-coverage",
                FAIL,
                f"REQUIRED_TARGET_UNKNOWN: {name} declares target {declared!r}; expected {list(REQUIRED_TARGETS)}",
            )
            bad += 1
            continue
        if stem != declared:
            report.add(
                "required-target-coverage",
                FAIL,
                f"REQUIRED_TARGET_MISMATCH: {name} must declare target {stem!r}, got {declared!r}",
            )
            bad += 1
            continue

        seen.add(stem)
        entries = data.get("entries")
        entry_counts[stem] = sum(1 for entry in entries if isinstance(entry, dict)) if isinstance(entries, list) else 0
        target_roots[stem] = resolve_target_root(str(data.get("target_root", ".")), oracle_dir)

    missing_files = [f"{target}.yaml" for target in REQUIRED_TARGETS if target not in seen]
    if missing_files:
        report.add(
            "required-target-coverage",
            FAIL,
            f"REQUIRED_TARGET_MISSING: missing required target classification file(s): {missing_files}",
        )
        bad += 1

    prov_path = os.path.join(oracle_dir, "BASELINE-PROVENANCE.yaml")
    if not os.path.isfile(prov_path):
        report.add("required-target-coverage", FAIL, f"REQUIRED_TARGET_PROVENANCE_MISSING: missing {prov_path}")
        bad += 1
    else:
        try:
            prov = load_yaml(prov_path)
        except yaml.YAMLError as exc:
            report.add(
                "required-target-coverage",
                FAIL,
                f"REQUIRED_TARGET_PROVENANCE_PARSE_ERROR: unparseable BASELINE-PROVENANCE.yaml: {exc}",
            )
            bad += 1
        else:
            targets = prov.get("targets") if isinstance(prov, dict) else None
            if not isinstance(targets, dict):
                report.add("required-target-coverage", FAIL, "REQUIRED_TARGET_PROVENANCE_MISSING: targets is not a mapping")
                bad += 1
            else:
                missing_prov = [target for target in REQUIRED_TARGETS if target not in targets]
                extra_prov = sorted(set(targets) - REQUIRED_TARGET_SET)
                if missing_prov:
                    report.add(
                        "required-target-coverage",
                        FAIL,
                        f"REQUIRED_TARGET_PROVENANCE_MISSING: provenance missing target(s): {missing_prov}",
                    )
                    bad += 1
                if extra_prov:
                    report.add(
                        "required-target-coverage",
                        FAIL,
                        f"REQUIRED_TARGET_UNKNOWN: provenance declares unknown target(s): {extra_prov}",
                    )
                    bad += 1
                for target in sorted(seen):
                    info = targets.get(target)
                    if not isinstance(info, dict):
                        continue
                    repo_path = info.get("repo_path")
                    if isinstance(repo_path, str) and repo_path != TBD and not repo_path.startswith("<"):
                        expected_root = os.path.normpath(repo_path)
                        if target_roots.get(target) != expected_root:
                            report.add(
                                "required-target-coverage",
                                FAIL,
                                f"REQUIRED_TARGET_ROOT_MISMATCH: {target}.yaml target_root "
                                f"{target_roots.get(target)} != provenance {expected_root}",
                            )
                            bad += 1
                    raw_findings = info.get("raw_findings")
                    if isinstance(raw_findings, int) and entry_counts.get(target) != raw_findings:
                        report.add(
                            "required-target-coverage",
                            FAIL,
                            f"REQUIRED_TARGET_COUNT_MISMATCH: {target}.yaml has {entry_counts.get(target)} "
                            f"classification(s), provenance expects {raw_findings}",
                        )
                        bad += 1

    if bad == 0:
        report.add(
            "required-target-coverage",
            PASS,
            f"required target coverage present for {expected_files}; classification targets align with provenance",
        )


def _raw_findings_live(binary: str, target_root: str) -> tuple[list[dict[str, Any]] | None, str, int]:
    if not os.path.isfile(binary):
        return None, f"binary not found at {binary}", 127
    try:
        proc = subprocess.run(
            [binary, "diagnose", target_root, "-f", "json", "-q"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=180,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return None, f"binary invocation failed: {exc}", 127
    if proc.returncode not in (0, 2):
        return None, f"binary exited {proc.returncode} (expected 0 or 2): {proc.stderr.strip()[:200]}", proc.returncode
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        return None, f"binary output not valid JSON: {exc}", proc.returncode
    if not isinstance(data, list):
        return None, "binary output is not a JSON array", proc.returncode
    return data, f"live binary (returncode={proc.returncode}): {len(data)} raw findings", proc.returncode


def load_raw_findings(
    report: Report,
    check_name: str,
    classification_path: str,
    data: dict[str, Any],
    oracle_dir: str,
    binary: str,
) -> tuple[list[dict[str, Any]] | None, str]:
    target_root = resolve_target_root(str(data.get("target_root", ".")), oracle_dir)
    raw_source = data.get("raw_source", "live")
    if raw_source != "live" and not is_selftest_path(classification_path):
        report.add(
            check_name,
            FAIL,
            f"{os.path.basename(classification_path)}: non-live raw_source rejected outside "
            f"tests/oracle/_selftest (raw_source must be live, got {raw_source!r})",
        )
        return None, "non-live raw_source rejected"
    if raw_source == "live":
        findings, note, _ = _raw_findings_live(binary, target_root)
        return findings, note
    raw_path = raw_source if os.path.isabs(raw_source) else os.path.join(oracle_dir, raw_source)
    try:
        with open(raw_path, "r", encoding="utf-8") as fh:
            findings = json.load(fh)
    except (OSError, json.JSONDecodeError) as exc:
        report.add(check_name, FAIL, f"{os.path.basename(classification_path)}: cannot load raw_source {raw_source}: {exc}")
        return None, "raw_source load error"
    if not isinstance(findings, list):
        report.add(check_name, FAIL, f"{os.path.basename(classification_path)}: raw_source is not a JSON array")
        return None, "raw_source shape error"
    return findings, f"planted raw ({os.path.basename(raw_path)}): {len(findings)} findings"


def check_count_equivalence(
    report: Report,
    files: list[tuple[str, dict[str, Any]]],
    oracle_dir: str,
    binary: str,
) -> None:
    if not files:
        report.add("count-equiv", SKIP, "no diagnostics/*.yaml yet (baseline not populated)")
        return
    total_matched = 0
    bad = 0
    for path, data in files:
        if "__parse_error__" in data:
            continue
        entries = data.get("entries")
        if not isinstance(entries, list):
            continue
        findings, note = load_raw_findings(report, "count-equiv", path, data, oracle_dir, binary)
        if findings is None:
            bad += 1
            continue

        raw_counter: Counter[tuple[str, str, int, str]] = Counter()
        raw_by_key: dict[tuple[str, str, int, str], list[dict[str, Any]]] = defaultdict(list)
        for finding in findings:
            if not isinstance(finding, dict):
                continue
            key = finding_key(finding)
            if key is not None:
                raw_counter[key] += 1
                raw_by_key[key].append(finding)

        cls_counter: Counter[tuple[str, str, int, str]] = Counter()
        entries_by_key: dict[tuple[str, str, int, str], list[dict[str, Any]]] = defaultdict(list)
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            key = entry_key(entry)
            if key is not None:
                cls_counter[key] += 1
                entries_by_key[key].append(entry)

        orphans = raw_counter - cls_counter
        extras = cls_counter - raw_counter
        duplicate_collapses = Counter(
            {
                key: raw_counter[key] - cls_counter[key]
                for key in raw_counter
                if raw_counter[key] > cls_counter.get(key, 0) and cls_counter.get(key, 0) > 0
            }
        )
        if orphans:
            report.add(
                "count-equiv",
                FAIL,
                f"{os.path.basename(path)}: {sum(orphans.values())} emitted finding(s) UNCLASSIFIED "
                f"(R001, multiset by pattern/path/line/symbol) e.g. {counter_examples(orphans)}",
            )
            bad += 1
        if duplicate_collapses:
            report.add(
                "count-equiv",
                FAIL,
                f"{os.path.basename(path)}: duplicate-collapse classification(s) detected; "
                f"one classification cannot cover multiple emitted findings e.g. "
                f"{counter_examples(duplicate_collapses)}",
            )
            bad += 1
        if extras:
            report.add(
                "count-equiv",
                FAIL,
                f"{os.path.basename(path)}: {sum(extras.values())} classification(s) map to NO emitted "
                f"finding (wrong-symbol/fabricated or duplicate-collapse) e.g. {counter_examples(extras)}",
            )
            bad += 1

        optional_bad = []
        raw_work = {k: list(v) for k, v in raw_by_key.items()}
        for entry in entries:
            if not isinstance(entry, dict):
                continue
            key = entry_key(entry)
            if key is None or not raw_work.get(key):
                continue
            finding = raw_work[key].pop(0)
            mismatch = optional_fingerprint_mismatches(entry, finding)
            if mismatch:
                optional_bad.append(f"{key}: {', '.join(mismatch)}")
        if optional_bad:
            report.add(
                "count-equiv",
                FAIL,
                f"{os.path.basename(path)}: optional stable fingerprint mismatch "
                f"(severity/confidence/message/range) e.g. {optional_bad[:3]}",
            )
            bad += 1

        if not orphans and not extras and not optional_bad:
            report.add(
                "count-equiv",
                PASS,
                f"{os.path.basename(path)}: {sum(raw_counter.values())} raw == "
                f"{sum(cls_counter.values())} classified ({note})",
            )
            total_matched += sum(raw_counter.values())
    if bad:
        report.add("count-equiv", FAIL, f"{bad} target(s) with classification/raw mismatch")
    elif total_matched:
        pass


def run_fixture_diagnose(binary: str, fixture_path: str, pattern: str) -> tuple[list[dict[str, Any]] | None, str]:
    findings, note, _ = _raw_findings_live_for_pattern(binary, fixture_path, pattern)
    return findings, note


def _raw_findings_live_for_pattern(
    binary: str, target_root: str, pattern: str
) -> tuple[list[dict[str, Any]] | None, str, int]:
    if not os.path.isfile(binary):
        return None, f"binary not found at {binary}", 127
    try:
        proc = subprocess.run(
            [binary, "diagnose", target_root, "--checks", pattern, "-f", "json", "-q"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=120,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        return None, f"binary invocation failed: {exc}", 127
    if proc.returncode not in (0, 2):
        return None, f"binary exited {proc.returncode}: {proc.stderr.strip()[:200]}", proc.returncode
    try:
        data = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        return None, f"binary output not valid JSON: {exc}", proc.returncode
    if not isinstance(data, list):
        return None, "binary output is not a JSON array", proc.returncode
    return data, f"live fixture ({proc.returncode=}): {len(data)} finding(s)", proc.returncode


def check_fixture_coverage(report: Report, oracle_dir: str, strict: bool, binary: str) -> None:
    cov_path = os.path.join(oracle_dir, "FIXTURE-COVERAGE.yaml")
    if not os.path.isfile(cov_path):
        report.add("fixture-coverage", FAIL, f"missing {cov_path}")
        return
    try:
        cov = load_yaml(cov_path)
    except yaml.YAMLError as exc:
        report.add("fixture-coverage", FAIL, f"unparseable FIXTURE-COVERAGE.yaml: {exc}")
        return
    if not isinstance(cov, dict) or not isinstance(cov.get("patterns"), dict):
        report.add("fixture-coverage", FAIL, "FIXTURE-COVERAGE.yaml missing 'patterns' mapping")
        return
    patterns = cov["patterns"]
    declared = set(patterns.keys())
    missing = CANONICAL_SET - declared
    extra = declared - CANONICAL_SET
    bad = 0
    if missing:
        report.add("fixture-coverage", FAIL, f"missing {len(missing)} pattern(s): {sorted(missing)}")
        bad += 1
    if extra:
        report.add("fixture-coverage", FAIL, f"unknown pattern(s) not in canonical 13: {sorted(extra)}")
        bad += 1
    for name, info in patterns.items():
        if name not in CANONICAL_SET:
            continue
        if not isinstance(info, dict):
            report.add("fixture-coverage", FAIL, f"{name}: entry is not a mapping")
            bad += 1
            continue
        status = info.get("status")
        if status not in VALID_COVERAGE_STATUS:
            report.add("fixture-coverage", FAIL, f"{name}: status '{status}' not in {sorted(VALID_COVERAGE_STATUS)}")
            bad += 1
            continue
        if strict and status == "implemented":
            fixtures = {
                "positive_fixture": True,
                "adversarial_fixture": False,
            }
            for kind, must_fire in fixtures.items():
                fp = info.get(kind)
                if not fp or fp == "none":
                    report.add("fixture-coverage", FAIL, f"{name}: {kind} missing under --strict-fixtures")
                    bad += 1
                    continue
                full = resolve_artifact_path(str(fp), oracle_dir)
                if not (os.path.isfile(full) or os.path.isdir(full)):
                    report.add("fixture-coverage", FAIL, f"{name}: {kind} path does not exist: {fp}")
                    bad += 1
                    continue
                findings, note = run_fixture_diagnose(binary, full, name)
                if findings is None:
                    report.add("fixture-coverage", FAIL, f"{name}: {kind} not executable: {note}")
                    bad += 1
                    continue
                fired = any(isinstance(f, dict) and f.get("pattern") == name for f in findings)
                if must_fire and not fired:
                    report.add("fixture-coverage", FAIL, f"{name}: positive_fixture did NOT fire ({note})")
                    bad += 1
                if (not must_fire) and fired:
                    report.add("fixture-coverage", FAIL, f"{name}: adversarial_fixture fired ({note})")
                    bad += 1
    n_status = sum(1 for n in patterns if n in CANONICAL_SET)
    if bad == 0:
        report.add(
            "fixture-coverage",
            PASS,
            f"all 13 patterns declared ({n_status} status entries)" + (" [strict executable]" if strict else ""),
        )


def check_comparator_rules(report: Report, comparator_path: str, binary: str) -> None:
    """Require rule registration and a semantic comparator run over planted truth."""
    if not os.path.isfile(comparator_path):
        report.add("comparator-rules", FAIL, f"comparator not found: {comparator_path}")
        return
    try:
        proc = subprocess.run(
            [sys.executable, comparator_path, "--list-rules"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=30,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        report.add("comparator-rules", FAIL, f"could not run comparator --list-rules: {exc}")
        return
    if proc.returncode != 0:
        report.add("comparator-rules", FAIL, f"comparator --list-rules exited {proc.returncode}: {proc.stderr.strip()[:200]}")
        return
    try:
        rules = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        report.add("comparator-rules", FAIL, f"comparator --list-rules did not emit valid JSON: {exc}")
        return
    if not isinstance(rules, list):
        report.add("comparator-rules", FAIL, "comparator --list-rules did not emit a JSON array")
        return
    ids = {r.get("id") for r in rules if isinstance(r, dict)}
    missing = [r for r in REQUIRED_RULE_IDS if r not in ids]
    if missing:
        report.add("comparator-rules", FAIL, f"comparator missing required rule(s): {missing}")
        return
    r007 = next((r for r in rules if isinstance(r, dict) and r.get("id") == "R007_FULL_VS_INCREMENTAL_MISMATCH"), {})
    if r007.get("status") != "deferred" or not r007.get("deferred_reason"):
        report.add("comparator-rules", FAIL, "R007 must be explicitly marked deferred with a reason")
        return

    sem_dir = os.path.join(repo_root_from_script(), "tests", "oracle", "_semtest")
    try:
        smoke = subprocess.run(
            [
                sys.executable,
                comparator_path,
                "--oracle-dir",
                sem_dir,
                "--flowspec-bin",
                binary,
                "--json",
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            timeout=180,
        )
    except (OSError, subprocess.SubprocessError) as exc:
        report.add("comparator-rules", FAIL, f"semantic comparator smoke could not run: {exc}")
        return
    if smoke.returncode == 0:
        report.add("comparator-rules", FAIL, "semantic comparator smoke unexpectedly passed planted violations")
        return
    try:
        verdict = json.loads(smoke.stdout)
    except json.JSONDecodeError as exc:
        report.add("comparator-rules", FAIL, f"semantic comparator smoke did not emit verdict JSON: {exc}")
        return
    got = {v["rule_id"] for v in verdict.get("violations", []) if isinstance(v, dict) and "rule_id" in v}
    required_smoke = set(SEMANTIC_SMOKE_RULE_IDS)
    missing_smoke = sorted(required_smoke - got)
    if missing_smoke:
        report.add("comparator-rules", FAIL, f"semantic comparator smoke missed rule application(s): {missing_smoke}")
        return
    report.add(
        "comparator-rules",
        PASS,
        f"all {len(REQUIRED_RULE_IDS)} rules registered; semantic smoke applied {sorted(got)}; R007 deferred",
    )


def check_traces(report: Report, oracle_dir: str) -> None:
    trace_files = sorted(glob.glob(os.path.join(oracle_dir, "traces", "*.yaml")))
    if not trace_files:
        report.add("traces", SKIP, "no traces/*.yaml yet (trace expectations not populated)")
        return
    valid_directions = frozenset({"forward", "backward"})
    total = 0
    bad = 0
    for path in trace_files:
        try:
            data = load_yaml(path)
        except yaml.YAMLError as exc:
            report.add("traces", FAIL, f"{os.path.basename(path)}: unparseable YAML: {exc}")
            bad += 1
            continue
        if not isinstance(data, dict):
            report.add("traces", FAIL, f"{os.path.basename(path)}: not a mapping")
            bad += 1
            continue
        if not data.get("symbol"):
            report.add("traces", FAIL, f"{os.path.basename(path)}: missing 'symbol'")
            bad += 1
        if data.get("direction") not in valid_directions:
            report.add("traces", FAIL, f"{os.path.basename(path)}: direction '{data.get('direction')}' not in {sorted(valid_directions)}")
            bad += 1
        hops = data.get("expected_hops")
        if not isinstance(hops, list) or not hops:
            report.add("traces", FAIL, f"{os.path.basename(path)}: missing/empty 'expected_hops'")
            bad += 1
            continue
        for j, hop in enumerate(hops):
            total += 1
            if not isinstance(hop, dict) or not hop.get("entity") or not hop.get("edge_kind"):
                report.add("traces", FAIL, f"{os.path.basename(path)} hop#{j}: each hop needs 'entity' + 'edge_kind'")
                bad += 1
    if bad == 0:
        report.add("traces", PASS, f"{len(trace_files)} trace file(s), {total} hops well-formed")
    else:
        report.add("traces", FAIL, f"{bad} trace-structure violation(s)")


def find_tbd(value: Any, prefix: str = "") -> list[str]:
    if value == TBD:
        return [prefix or "<root>"]
    if isinstance(value, dict):
        out: list[str] = []
        for key, item in value.items():
            child = f"{prefix}.{key}" if prefix else str(key)
            out.extend(find_tbd(item, child))
        return out
    if isinstance(value, list):
        out = []
        for i, item in enumerate(value):
            out.extend(find_tbd(item, f"{prefix}[{i}]"))
        return out
    return []


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as fh:
        for chunk in iter(lambda: fh.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def git_output(args: list[str], cwd: str) -> tuple[str | None, str]:
    try:
        proc = subprocess.run(args, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=30)
    except (OSError, subprocess.SubprocessError) as exc:
        return None, str(exc)
    if proc.returncode != 0:
        return None, proc.stderr.strip()
    return proc.stdout.strip(), ""


def pattern_counts(findings: list[dict[str, Any]]) -> dict[str, int]:
    counts: Counter[str] = Counter()
    for finding in findings:
        if isinstance(finding, dict) and finding.get("pattern"):
            counts[str(finding["pattern"])] += 1
    return dict(sorted(counts.items()))


def check_provenance(report: Report, oracle_dir: str, binary: str) -> None:
    prov_path = os.path.join(oracle_dir, "BASELINE-PROVENANCE.yaml")
    if not os.path.isfile(prov_path):
        report.add("provenance", FAIL, f"missing {prov_path}")
        return
    try:
        prov = load_yaml(prov_path)
    except yaml.YAMLError as exc:
        report.add("provenance", FAIL, f"unparseable BASELINE-PROVENANCE.yaml: {exc}")
        return
    if not isinstance(prov, dict):
        report.add("provenance", FAIL, "BASELINE-PROVENANCE.yaml is not a mapping")
        return
    missing_keys = [k for k in PROVENANCE_TOP_KEYS if k not in prov]
    if missing_keys:
        report.add("provenance", FAIL, f"missing top-level key(s): {missing_keys}")
        return
    if not has_real_diagnostics(oracle_dir):
        report.add("provenance", PASS, "structure present (no production diagnostics yet)")
        return

    bad = 0
    tbd_paths = find_tbd(prov)
    if tbd_paths:
        report.add("provenance", FAIL, f"{TBD} remains after diagnostics exist: {tbd_paths[:8]}")
        bad += 1

    repo_root = repo_root_from_script()
    head, err = git_output(["git", "rev-parse", "HEAD"], repo_root)
    if head is None or prov.get("git", {}).get("flowspec_sha") != head:
        report.add("provenance", FAIL, f"git.flowspec_sha mismatch: expected live {head or err}")
        bad += 1
    status, err = git_output(["git", "status", "--short"], repo_root)
    if status is None:
        report.add("provenance", FAIL, f"cannot read git status: {err}")
        bad += 1
    else:
        clean = status == ""
        git_info = prov.get("git", {})
        if git_info.get("flowspec_status_clean") is not clean or str(git_info.get("flowspec_status_summary", "")) != status:
            report.add("provenance", FAIL, "git status clean/summary does not match live git status --short")
            bad += 1

    if not os.path.isfile(binary):
        report.add("provenance", FAIL, f"binary missing for provenance check: {binary}")
        bad += 1
    else:
        live_sha = sha256_file(binary)
        if prov.get("binary", {}).get("sha256") != live_sha:
            report.add("provenance", FAIL, "binary.sha256 does not match live sha256sum")
            bad += 1
        try:
            version = subprocess.run([binary, "--version"], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, timeout=30)
        except (OSError, subprocess.SubprocessError) as exc:
            report.add("provenance", FAIL, f"cannot run binary --version: {exc}")
            bad += 1
        else:
            if version.returncode != 0 or prov.get("version") != version.stdout.strip():
                report.add("provenance", FAIL, "version does not match live flowspec --version")
                bad += 1

    targets = prov.get("targets")
    if not isinstance(targets, dict):
        report.add("provenance", FAIL, "targets is not a mapping")
        bad += 1
    else:
        for name, target in targets.items():
            if not isinstance(target, dict):
                report.add("provenance", FAIL, f"target {name}: not a mapping")
                bad += 1
                continue
            repo_path = target.get("repo_path")
            if not isinstance(repo_path, str):
                report.add("provenance", FAIL, f"target {name}: missing repo_path")
                bad += 1
                continue
            findings, note, exit_code = _raw_findings_live(binary, repo_path)
            if findings is None:
                report.add("provenance", FAIL, f"target {name}: live diagnose failed: {note}")
                bad += 1
                continue
            if target.get("diagnose_exit_code") != exit_code:
                report.add("provenance", FAIL, f"target {name}: diagnose_exit_code mismatch")
                bad += 1
            if target.get("raw_findings") != len(findings):
                report.add("provenance", FAIL, f"target {name}: raw_findings mismatch live {len(findings)}")
                bad += 1
            if target.get("by_pattern") != pattern_counts(findings):
                report.add("provenance", FAIL, f"target {name}: by_pattern mismatch live counts")
                bad += 1
            repo_sha = target.get("repo_sha")
            if repo_sha:
                live_repo_sha, err = git_output(["git", "rev-parse", "HEAD"], repo_path)
                if live_repo_sha is None or repo_sha != live_repo_sha:
                    report.add("provenance", FAIL, f"target {name}: repo_sha mismatch {live_repo_sha or err}")
                    bad += 1
    if bad == 0:
        report.add("provenance", PASS, "filled provenance matches live git, binary, version, and raw counts")


def run_all(oracle_dir: str, comparator_path: str, binary: str, strict_fixtures: bool) -> Report:
    label = f"oracle @ {os.path.relpath(oracle_dir, repo_root_from_script())}"
    report = Report(label)
    binary = resolve_binary(binary)
    files = load_classification_files(oracle_dir)
    check_schema(report, files)
    check_antifabrication(report, files, oracle_dir)
    check_required_target_coverage(report, files, oracle_dir)
    check_count_equivalence(report, files, oracle_dir, binary)
    check_fixture_coverage(report, oracle_dir, strict_fixtures, binary)
    check_comparator_rules(report, comparator_path, binary)
    check_traces(report, oracle_dir)
    check_provenance(report, oracle_dir, binary)
    return report


def run_self_test(repo_root: str) -> int:
    base = os.path.join(repo_root, "tests", "oracle", "_selftest")
    samples = [
        ("GOOD", os.path.join(base, "good"), True, False),
        ("BAD", os.path.join(base, "bad"), False, True),
        ("PROD", os.path.join(repo_root, "tests", "oracle", "_production_bad"), False, False),
    ]
    comparator = os.path.join(repo_root, "scripts", "flowspec-oracle.py")
    binary = resolve_binary(os.path.join(repo_root, "target", "release", "flowspec"))

    print("=" * 72)
    print("SELF-TEST: validator must accept the clean sample and flag planted samples")
    print("=" * 72)

    problems: list[str] = []
    rows: list[tuple[str, str, str, int, int]] = []

    for label, sample_dir, must_pass, strict in samples:
        rel = os.path.relpath(sample_dir, repo_root) if os.path.isdir(sample_dir) else sample_dir
        if not os.path.isdir(sample_dir):
            problems.append(f"{label} sample dir missing: {rel}")
            rows.append((label, rel, "missing", 0, 0))
            continue
        rep = run_all(sample_dir, comparator, binary=binary, strict_fixtures=strict)
        n_ok = sum(1 for _, s, _ in rep.results if s == PASS)
        n_bad = sum(1 for _, s, _ in rep.results if s == FAIL)
        verdict = "accepted" if not rep.failed else "rejected"
        rows.append((label, rel, verdict, n_ok, n_bad))
        if must_pass and rep.failed:
            problems.append(f"{label} sample should be accepted but was rejected")
        if (not must_pass) and (not rep.failed):
            problems.append(f"{label} sample should be rejected but was accepted")
        for marker_name, check_name, needle in EXPECTED_SELFTEST_FAILURE_MARKERS.get(label, []):
            observed = any(
                status == FAIL and check == check_name and needle in detail
                for check, status, detail in rep.results
            )
            if not observed:
                problems.append(
                    f"{label} sample did not prove {marker_name}: "
                    f"missing {check_name} failure containing {needle!r}"
                )

    print()
    total_bad = 0
    for label, rel, verdict, n_ok, n_bad in rows:
        total_bad += n_bad
        print(f"  {label:4s} sample ({rel}): {verdict} - {n_ok} ok, {n_bad} violation(s)")

    print(f"\n  planted violation count: {total_bad}")
    print("\n" + "=" * 72)
    if problems:
        print("SELF-TEST: REJECT - validator behavior is incorrect:")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("SELF-TEST: PASS - clean sample accepted, planted samples caught.")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        prog="validate-oracle-artifacts",
        description="The semantic oracle-contract gate.",
    )
    parser.add_argument("--root", default=None, help="flowspec repo root (default: inferred).")
    parser.add_argument("--self-test", action="store_true", help="Run validator self-test.")
    parser.add_argument(
        "--strict-fixtures",
        action="store_true",
        help="Run implemented fixture positives/adversarials through flowspec.",
    )
    parser.add_argument("--flowspec-bin", default=None, help="Override flowspec binary path.")
    args = parser.parse_args(argv)

    repo_root = os.path.abspath(args.root) if args.root else repo_root_from_script()
    if args.self_test:
        return run_self_test(repo_root)

    oracle_dir = os.path.join(repo_root, "tests", "oracle")
    comparator = os.path.join(repo_root, "scripts", "flowspec-oracle.py")
    binary = resolve_binary(args.flowspec_bin or os.path.join(repo_root, "target", "release", "flowspec"))
    report = run_all(oracle_dir, comparator, binary, args.strict_fixtures)
    report.print()
    print(f"\n{report.summary_line()}")
    if report.failed:
        print("RESULT: FAIL - oracle contract violations above must be fixed before release.")
        return 1
    print("RESULT: PASS - oracle contract semantic gate is sound.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
