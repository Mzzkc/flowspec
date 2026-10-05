#!/usr/bin/env python3
"""Refuse absent release gate implementations; this is not a substitute gate."""
import argparse
import json
from pathlib import Path
import subprocess
import sys

ROOT=Path(__file__).resolve().parents[1]
REQUIRED={
    'coverage': ['scripts/coverage.sh'],
    'oracle': ['scripts/oracle/run.py','scripts/oracle/mutations.py','tests/oracle/corpus.lock.json'],
    'security': ['scripts/check-security.py','scripts/check-dependencies.sh'],
    'reproducibility': ['scripts/check-reproducibility.sh'],
    'cache-equivalence': ['scripts/check-cache-equivalence.py'],
    'test-inventory': ['scripts/check-test-inventory.py'],
}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('gate',choices=REQUIRED)
    p.add_argument('--root',type=Path,default=ROOT)
    p.add_argument('--out',type=Path,required=True)
    args=p.parse_args()
    print('import:',Path(__file__).resolve(),file=sys.stderr)
    missing=[];unsafe=[]
    for relative in REQUIRED[args.gate]:
        path=args.root/relative
        if not path.is_file():missing.append(relative)
        elif any(a.is_symlink() for a in (path,*path.parents)):unsafe.append(relative)
    status='HOLD' if missing or unsafe else 'PREREQUISITES_PRESENT_NOT_QUALIFIED'
    args.out.mkdir(parents=True,exist_ok=True)
    report={'schema':'flowspec.ci-prerequisites.v1','gate':args.gate,'status':status,'missing':missing,'unsafe':unsafe,'required':REQUIRED[args.gate], 'explanation':'Presence never certifies semantic gate correctness; CI must invoke the real gate and independent review must accept it.'}
    (args.out/(args.gate+'-prerequisite.json')).write_text(json.dumps(report,sort_keys=True,indent=2)+'\n')
    print(json.dumps(report))
    return 1 if missing or unsafe else 0
if __name__=='__main__':sys.exit(main())
