#!/usr/bin/env python3
"""Execute the AST oracle against a pinned flowspec binary and retain evidence."""
import argparse
import os
import hashlib
import json
import subprocess
import time
from datetime import datetime, timezone
from pathlib import Path

# Reports and evidence go to --out (default: the current directory), never beside the script.
BASE = Path(os.environ.get('FLOWSPEC_ORACLE_OUT', '.')).resolve()
BINARY = Path(os.environ.get('FLOWSPEC_BIN', str(Path(__file__).resolve().parents[2] / 'target/release/flowspec')))
PLAIN = False


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def cli_symbol(identity):
    """The root-relative path keeps same-named files in different directories apart."""
    return identity


def caller_id(text):
    return text.split('/')[-1]


def score(case, flows):
    certain = case['certain']
    candidate = case['candidate']
    known = {caller_id(x['caller']) for x in certain}
    possible = {caller_id(x['caller']) for x in candidate}
    claimed = set()
    for flow in flows if isinstance(flows, list) else []:
        if isinstance(flow, dict):
            steps = flow.get('steps') or []
            # Backward traces include transitive ancestors at exit. The first
            # step is the immediate caller and is the only valid comparison.
            if steps and isinstance(steps[0].get('entity'), str):
                claimed.add(caller_id(steps[0]['entity']))
    missed = [x for x in certain if caller_id(x['caller']) not in claimed]
    false = sorted(claimed - known - possible)
    overlapping = sorted((claimed - known) & possible)
    return {'found_callers': len(known & claimed), 'certain_callers': len(known),
            'missed': missed, 'false_positive_callers': false,
            'candidate_overlaps': overlapping, 'claimed': sorted(claimed)}


def score_response(case, raw):
    """Keep a failed trace out of caller accuracy denominators."""
    code = raw['exit_code']
    try:
        flows = json.loads(raw['stdout'])
        valid = isinstance(flows, list)
    except json.JSONDecodeError:
        valid = False
        flows = None
    if code == 0 and valid:
        return {**score(case, flows), 'failure_kind': None}
    kind = ('killed_under_cap' if code in (-9, 137) else 'timeout' if code == 124
            else 'trace_error' if code else 'invalid_json')
    return {'found_callers': 0, 'certain_callers': len({caller_id(x['caller']) for x in case['certain']}),
            'missed': [], 'false_positive_callers': [], 'candidate_overlaps': [],
            'claimed': [], 'failure_kind': kind}


def seed_omission(case, flows):
    """Delete one real caller from a copied answer before it reaches score()."""
    victim = caller_id(case['certain'][0]['caller'])
    altered = [f for f in flows if not f.get('steps') or caller_id(f['steps'][0].get('entity', '')) != victim]
    if len(altered) == len(flows):
        raise RuntimeError(f'cannot seed omission: {victim} absent from answer')
    return altered, victim


def run_case(case, root, evidence, index):
    symbol = cli_symbol(case['symbol'])
    # Hard memory cap added by Legion (conductor) 2026-11-10 after an uncapped run exhausted host memory at 11:27Z.
    # Every trace runs under a hard memory cap and a wall clock. On a host
    # without a user systemd (CI containers), --plain runs the binary with
    # only its own --memory-limit budget and the timeout.
    wrapper = [] if PLAIN else ['systemd-run', '--user', '--scope', '-q', '-p', 'MemoryMax=2G', '-p', 'MemorySwapMax=0']
    argv = wrapper + ['timeout', '120', str(BINARY), 'trace', '--symbol', symbol,
            '--direction', 'backward', '--format', 'json', '--memory-limit', '2G', str(root)]
    start = time.monotonic()
    proc = subprocess.run(argv, text=True, capture_output=True)
    elapsed = time.monotonic() - start
    raw = {'argv': argv, 'exit_code': proc.returncode, 'seconds': round(elapsed, 3),
           'stdout': proc.stdout, 'stderr': proc.stderr}
    (evidence / f'{index:02d}.json').write_text(json.dumps(raw, indent=2) + '\n')
    return {**score_response(case, raw), 'symbol': case['symbol'], 'query': symbol,
            'exit_code': proc.returncode, 'seconds': round(elapsed, 3),
            'raw': f'evidence/{evidence.name}/{index:02d}.json'}


def report(data):
    rows = data['results']
    lines = [f'# Python caller oracle — {data["label"]}', '',
             f'UTC: {data["time"]}', f'Binary SHA-256: `{data["binary_sha256"]}`',
             f'AST source root: `{data["root"]}`',
             'Scoring unit: caller entity. A trace exit is matched to the AST enclosing function; '
             'a single claimed function cannot prove every call expression inside it. '
             'Unresolved receiver calls are candidates, not certain false positives.',
             f'Coverage: {len(rows)}/{data.get("requested_count", len(rows))} symbols executed; '
             'a killed or timed-out trace is unscored, not a missed caller.', '',
             '| Symbol | Certain callers found/total | Missed sites | Wrong callers | Candidate overlaps | Seconds | Exit | Trace failure |',
             '| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |']
    for r in rows:
        scored = not r.get('failure_kind')
        found = f'{r["found_callers"]}/{r["certain_callers"]}' if scored else 'unscored'
        misses = str(len(r['missed'])) if scored else '—'
        wrong = str(len(r['false_positive_callers'])) if scored else '—'
        overlaps = str(len(r['candidate_overlaps'])) if scored else '—'
        lines.append(f'| `{r["symbol"]}` | {found} | {misses} | {wrong} | {overlaps} | {r["seconds"]:.3f} | {r["exit_code"]} | {r.get("failure_kind") or "—"} |')
    lines += ['', '## Certain call sites missed', '']
    for r in rows:
        for m in r['missed']:
            lines.append(f'- `{r["symbol"]}` misses `{m["file"]}:{m["line"]}` in `{m["caller"]}` ({m["reason"]}).')
    if not any(r['missed'] for r in rows):
        lines.append('- None scored.' if any(r.get('failure_kind') for r in rows) else '- None.')
    lines += ['', '## Wrongly claimed caller entities', '']
    for r in rows:
        for x in r['false_positive_callers']:
            lines.append(f'- `{r["symbol"]}` wrongly claims `{x}`; no AST call of that name in a compatible enclosing function.')
    if not any(r['false_positive_callers'] for r in rows):
        lines.append('- None proved by this conservative oracle.')
    failures = sum(len(r['missed']) + len(r['false_positive_callers']) for r in rows)
    error_kinds = {kind: sum(r.get('failure_kind') == kind for r in rows)
                   for kind in ('killed_under_cap', 'timeout', 'trace_error', 'invalid_json')}
    errors = sum(error_kinds.values())
    lines += ['', f'VERDICT: {"UNRELIABLE" if failures or errors else "RELIABLE WITH STATIC LIMITS"} for the sampled Python callers; {failures} proven mismatches and {errors} unscored trace failures ({error_kinds}).', '']
    if (BASE / 'diagnose.md').exists():
        diagnose_lines = (BASE / 'diagnose.md').read_text().splitlines()
        table_start = next(i for i, line in enumerate(diagnose_lines) if line.startswith('| Area |'))
        heading = ('## Diagnose adjudication (30 source-read findings; prior baseline snapshot)'
                   if data['label'] != 'baseline' else '## Diagnose adjudication (30 source-read findings)')
        note = (['These diagnose findings were not rerun against this release binary.', '']
                if data['label'] != 'baseline' else [])
        lines += [heading, '', *note, *diagnose_lines[table_start:], '',
                  'Full source evidence: `diagnose.md` and `diagnose-adjudications.json`.', '']
    return '\n'.join(lines)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument('--manifest', type=Path, default=Path('cases.json'))
    ap.add_argument('--label', default='baseline')
    ap.add_argument('--plain', action='store_true', help='no systemd-run cgroup wrapper (CI hosts)')
    ap.add_argument('--raw-label', help='evidence directory to read for --rescore')
    ap.add_argument('--seed-wrong-answer', action='store_true')
    ap.add_argument('--rescore', action='store_true', help='read retained raw responses instead of invoking flowspec')
    args = ap.parse_args()
    global PLAIN
    PLAIN = args.plain
    if args.seed_wrong_answer and not args.rescore:
        ap.error('--seed-wrong-answer requires --rescore so raw evidence remains intact')
    manifest = json.loads(args.manifest.read_text())
    stamp = datetime.now(timezone.utc).isoformat(timespec='seconds')
    before = digest(BINARY)
    evidence = BASE / 'evidence' / args.label
    evidence.mkdir(parents=True, exist_ok=True)
    raw_evidence = BASE / 'evidence' / (args.raw_label or args.label)
    results = []
    aborted = False
    for i, case in enumerate(manifest['cases']):
        if args.rescore:
            raw = json.loads((raw_evidence / f'{i:02d}.json').read_text())
            result = score_response(case, raw)
            if args.seed_wrong_answer and i == 0:
                if result['failure_kind']:
                    raise RuntimeError('cannot seed omission into a failed trace')
                flows = json.loads(raw['stdout'])
                flows, victim = seed_omission(case, flows)
                print(f'SEED: removed actual caller {victim} from scorer input', flush=True)
                result = {**score(case, flows), 'failure_kind': None}
            result = {**result, 'symbol': case['symbol'],
                      'query': cli_symbol(case['symbol']), 'exit_code': raw['exit_code'],
                      'seconds': raw['seconds'], 'raw': f'evidence/{raw_evidence.name}/{i:02d}.json'}
        else:
            result = run_case(case, manifest['root'], evidence, i)
        results.append(result)
        print(f'{i+1}/{len(manifest["cases"])} {case["symbol"]}: {result["found_callers"]}/{result["certain_callers"]} certain callers, {len(result["false_positive_callers"])} wrong, exit {result["exit_code"]}', flush=True)
        if digest(BINARY) != before:
            raise SystemExit('INVALID BATCH: binary changed while scoring')
        if not args.rescore and result['failure_kind'] in ('killed_under_cap', 'timeout'):
            aborted = True
            print(f'ABORT: {result["failure_kind"]} on {case["symbol"]}; remaining traces not run under the same cap', flush=True)
            break
    data = {'label': args.label, 'time': stamp, 'root': manifest['root'],
            'binary_sha256': before, 'requested_count': len(manifest['cases']), 'results': results}
    (evidence / 'results.json').write_text(json.dumps(data, indent=2) + '\n')
    output = BASE / f'{args.label}.md'
    output.write_text(report(data))
    if args.seed_wrong_answer:
        print(f'SEED RESULT: {output}; {len(results[0]["missed"])} missed AST sites detected')
        raise SystemExit(1 if results[0]['missed'] else 0)
    print(f'REPORT: {output}; SHA-256 {digest(BINARY)}')
    if aborted:
        raise SystemExit(2)


if __name__ == '__main__':
    main()
