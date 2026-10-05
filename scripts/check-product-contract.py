#!/usr/bin/env python3
"""Check active CI commands and generated live-registry documentation."""
import argparse
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile

ROOT=Path(__file__).resolve().parents[1]
PINS={
 'actions/checkout':'3d3c42e5aac5ba805825da76410c181273ba90b1',
 'actions/upload-artifact':'043fb46d1a93c77aae656e7c1c64a875d1fc6a0a',
 'actions/setup-python':'5fda3b95a4ea91299a34e894583c3862153e4b97',
}


def validate_ci(text):
    # Accept the deliberately restricted workflow syntax tracked here. Refuse
    # alternate YAML keys, aliases, flow syntax and conditionals rather than
    # treating unparsed syntax as equivalent to the checked commands.
    if any(word in text for word in ('continue-on-error','pull_request_target','secrets.','!include')):
        raise ValueError('unsafe event, secret exposure or soft pass')
    active='\n'.join(line for line in text.splitlines() if line.strip() and not line.lstrip().startswith('#'))
    if 'permissions:\n  contents: read' not in active or re.search(r'^\s+(?:permissions|continue-on-error):',active,re.M):
        raise ValueError('least permissions not preserved')
    if re.search(r'^\s*[^#\n]*[&*][A-Za-z_]',active,re.M):
        raise ValueError('YAML aliases/anchors not admitted')
    conditions=re.findall(r'^\s+(?:- )?if:\s*(.+)$',active,re.M)
    if any(condition!='always()' for condition in conditions):
        raise ValueError('conditional gate bypass')
    uses=re.findall(r'^\s+- uses: ([^\s#]+)',active,re.M)
    if not uses or any('@' not in item or item.rsplit('@',1)[1]!=PINS.get(item.rsplit('@',1)[0]) for item in uses):
        raise ValueError('unverified or moving action pin')
    if '${{' in active and any('secrets' in expr for expr in re.findall(r'\$\{\{(.*?)\}\}',active)):
        raise ValueError('secret expression')
    if 'python-version: 3.11.15' not in active:
        raise ValueError('Python version unpinned')
    # Required commands must be actual standalone commands in run blocks.
    commands=[]
    lines=active.splitlines(); in_run=False;run_indent=0
    for line in lines:
        indent=len(line)-len(line.lstrip())
        m=re.match(r'\s+run: (.*)$',line)
        if m:
            if m.group(1)=='|':in_run=True;run_indent=indent
            else:commands.append(m.group(1));in_run=False
        elif in_run:
            if indent>run_indent:commands.append(line.strip())
            else:in_run=False
    if any(re.search(r"^(?:exit|return|exec|source|eval|\.)\b|(?:^|[;&|])\s*(?:exit|return)\b",cmd) for cmd in commands):
        raise ValueError('early termination or opaque control can hide required gates')
    command_set=set(commands)
    required={
      'rustup toolchain install 1.91.0 --profile minimal --component rustfmt --component clippy',
      'cargo +1.91.0 fmt --all --check',
      'cargo +1.91.0 clippy --locked --workspace --all-targets --all-features -- -D warnings',
      'cargo +1.91.0 test --locked --workspace --all-features 2>&1 | tee evidence/tests.log',
      'python3 scripts/check-test-custody.py --root . --policy tests/test-custody-policy.json --check-markers 2>&1 | tee evidence/custody-markers.log',
      'python3 scripts/check-test-custody.py --root . --policy tests/test-custody-policy.json --baseline tests/test-custody-baseline.json 2>&1 | tee evidence/custody-baseline.log',
      'python3 scripts/check-test-inventory.py --log evidence/tests.log --out evidence',
      'bash scripts/coverage.sh --locked --out evidence/coverage',
      'python3 scripts/oracle/run.py --binary target/release/flowspec --lock tests/oracle/corpus.lock.json --out evidence/oracle',
      'python3 scripts/oracle/mutations.py --binary target/release/flowspec --lock tests/oracle/corpus.lock.json --out evidence/mutations',
      'python3 scripts/check-security.py --binary target/release/flowspec --out evidence/security',
      'bash scripts/check-dependencies.sh --locked --out evidence/dependencies',
      'python3 scripts/check-cache-equivalence.py --binary target/release/flowspec --out evidence/cache-equivalence',
      'bash scripts/check-reproducibility.sh --locked --out evidence/reproducibility',
      'python3 scripts/check-product-contract.py --binary target/release/flowspec --check --require-corpus',
      'python3 -m unittest discover -s tests/skill -p "test_*.py"',
      'set -euo pipefail',
    }
    required.update('python3 scripts/check-ci-prerequisites.py '+gate+' --out evidence' for gate in ['coverage','oracle','security','reproducibility','cache-equivalence','test-inventory'])
    missing=sorted(required-command_set)
    if missing:raise ValueError('missing active CI gates: '+repr(missing))
    if any(re.search(r'\b(?:cargo|python3|bash)\b.*(?:\|\|\s*(?:true|:)|--(?:ignored|skip|exact)|--\s+[A-Za-z])',cmd) for cmd in commands):
        raise ValueError('filtered or softened validation command')
    # Bash piped test commands propagate the test status, not just tee status.
    if 'shell: bash' not in active or 'if: always()' not in active or 'path: evidence' not in active or 'if-no-files-found: error' not in active:
        raise ValueError('failure-safe artifact custody missing')
    if 'persist-credentials: false' not in active or 'runs-on: ubuntu-24.04' not in active:
        raise ValueError('runner or credential policy missing')


def read_catalog(binary):
    r=subprocess.run([str(binary.resolve()),'registry'],capture_output=True,text=True,timeout=30)
    if r.returncode:raise ValueError('registry command failed')
    catalog=json.loads(r.stdout)
    if len(catalog['patterns'])!=13 or len({p['id'] for p in catalog['patterns']})!=13 or not all(p['default_enabled'] for p in catalog['patterns']):
        raise ValueError('registry does not declare exactly thirteen defaults')
    if {l['id'] for l in catalog['languages'] if l['default_enabled']}!={'python','javascript','typescript','rust'}:
        raise ValueError('four launch language defaults disagree')
    return catalog


def render_tables(catalog):
    preamble=f"Registry digest: `{catalog['digest']}`. Generated by `scripts/check-product-contract.py`; registration is not semantic or launch qualification. The immutable four-language exact oracle remains required.\n\n"
    diagnostics='# Registered diagnostics\n\n'+preamble+'| ID | Default | Required capabilities | Applicability |\n| --- | --- | --- | --- |\n'
    for p in catalog['patterns']:
        diagnostics+=f"| `{p['id']}` | {'enabled' if p['default_enabled'] else 'disabled'} | {', '.join(p['required_capabilities'])} | {p['applicability']} |\n"
    languages='# Registered language support\n\n'+preamble+'| Language | Extensions | Adapter | Grammar | Capabilities | Declared limitations |\n| --- | --- | --- | --- | --- | --- |\n'
    for l in catalog['languages']:
        languages+=f"| {l['id']} | {', '.join(l['extensions'])} | {l['adapter_version']} | {l['grammar_version']} | {', '.join(l['capabilities'])} | {', '.join(l['limitations']) or 'none declared; completeness must still be checked'} |\n"
    languages+='\nAdd a language through the instance-local registry adapter seam described in [registry](registry.md). Require original-source ranges, recovery/loss observation, declared capabilities, and conformance proof. Missing capabilities or unsupported constructs cannot be counted as clean negatives. TypeScript preprocessing is explicitly limited when the live registry reports it; a joined native adapter requires regenerating this table and fresh evidence.\n'
    return {'docs/diagnostics.md':diagnostics,'docs/language-support.md':languages}


def check_tables(root,catalog):
    for relative,text in render_tables(catalog).items():
        if (root/relative).read_text()!=text:raise ValueError('live registry documentation drift: '+relative)


def require_corpus(root):
    path=root/'tests/oracle/corpus.lock.json'
    if not path.is_file():raise ValueError('HOLD exact sealed corpus lock absent')
    # This component does not invent/adjudicate a lock schema. The independently
    # owned exact oracle validates it. Presence cannot stand in for oracle proof.
    runner=root/'scripts/oracle/run.py'
    schema=root/'scripts/oracle/schema.py'
    if not runner.is_file() or not schema.is_file():raise ValueError('HOLD exact oracle/schema absent')
    raise ValueError('HOLD corpus-aware documentation generation awaits independently admitted oracle schema integration')


def check_links(root):
    for relative in ['README.md','CONTRIBUTING.md','docs/reliability.md','docs/security.md','docs/agent-workflow.md','docs/diagnostics.md','docs/language-support.md']:
        p=root/relative
        for target in re.findall(r'\]\(([^)]+)\)',p.read_text()):
            if ':' in target or target.startswith('#'):continue
            if not (p.parent/target.split('#')[0]).is_file():raise ValueError('broken product link: '+relative+' -> '+target)


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary',type=Path)
    p.add_argument('--check',action='store_true')
    p.add_argument('--write',action='store_true')
    p.add_argument('--require-corpus',action='store_true')
    p.add_argument('--ci-only',action='store_true')
    args=p.parse_args()
    print('import:',Path(__file__).resolve(),file=sys.stderr)
    try:
        validate_ci((ROOT/'.github/workflows/ci.yml').read_text())
        if args.ci_only:print('CI contract structure accepted; gate execution not implied');return 0
        if not args.binary:raise ValueError('--binary is required for live public agreement')
        catalog=read_catalog(args.binary)
        if args.write:
            for relative,text in render_tables(catalog).items():(ROOT/relative).write_text(text)
        check_tables(ROOT,catalog);check_links(ROOT)
        for command in ['analyze','diagnose','trace','diff']:
            help_result=subprocess.run([str(args.binary.resolve()),command,'--help'],capture_output=True,text=True,timeout=15)
            if help_result.returncode or '--format' not in help_result.stdout:raise ValueError('CLI command help disagrees: '+command)
        if args.require_corpus:require_corpus(ROOT)
        print('Live catalog/table/help agreement accepted; corpus agreement and release qualification not implied')
        return 0
    except (ValueError,OSError,KeyError,subprocess.SubprocessError) as exc:
        print(str(exc),file=sys.stderr);return 1
if __name__=='__main__':sys.exit(main())
