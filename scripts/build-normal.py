#!/usr/bin/env python3
"""Build a clean source-bound normal CLI using a leased shared Cargo target.

Usage: python3 scripts/build-normal.py --lease PATH --output PATH
The output must be outside the source tree. No timestamps are touched.
"""
import argparse
import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--lease', required=True, type=Path)
    parser.add_argument('--output', required=True, type=Path)
    args = parser.parse_args()
    root = Path(__file__).resolve().parents[1]
    def git(*words):
        return subprocess.check_output(['git', *words], cwd=root, text=True).strip()
    if git('status', '--porcelain'):
        raise SystemExit('source-bound normal build requires a clean checkout')
    sha, tree = git('rev-parse', 'HEAD'), git('rev-parse', 'HEAD^{tree}')
    output = args.output.resolve()
    if output.is_relative_to(root):
        raise SystemExit('immutable build output must be outside the source tree')
    output.parent.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env['FLOWSPEC_BUILD_SOURCE_SEAL'] = f'{root}:{sha}:{tree}'
    with args.lease.open('a') as lease:
        fcntl.flock(lease, fcntl.LOCK_EX)
        metadata = json.loads(subprocess.check_output(
            ['cargo', 'metadata', '--offline', '--locked', '--no-deps', '--format-version', '1'], cwd=root, env=env))
        subprocess.run(['cargo', 'build', '--offline', '--locked', '-p', 'flowspec-cli'], cwd=root, env=env, check=True)
        if git('rev-parse', 'HEAD') != sha or git('status', '--porcelain'):
            raise SystemExit('source changed during build')
        shutil.copyfile(Path(metadata['target_directory']) / 'debug/flowspec', output)
        output.chmod(0o755)
        receipt = dict(candidate_sha=sha, source_tree=tree, source_root=str(root), binary_path=str(output),
                       binary_sha256=hashlib.sha256(output.read_bytes()).hexdigest(),
                       build_source_seal=env['FLOWSPEC_BUILD_SOURCE_SEAL'], lease=str(args.lease.resolve()),
                       copy_under_shared_cargo_lease=True, timestamp_touch=False)
        output.with_suffix(output.suffix + '.binding.json').write_text(json.dumps(receipt, indent=2) + '\n')
        print(json.dumps(receipt))


if __name__ == '__main__':
    main()
