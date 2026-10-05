#!/usr/bin/env python3
"""Deterministic local distribution. Identity is recorded; release approval is not."""
import argparse
import hashlib
import io
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import sys
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
SKILL_FILES = ('SKILL.md', 'references/examples.md', 'references/report-contract.md')
MAX_SKILL_BYTES = 12000


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def regular_bytes(path, max_bytes=None):
    # Never follow a file or ancestor symlink at this distribution boundary.
    if any(p.is_symlink() for p in (path, *path.parents)):
        raise ValueError(f'symlink input refused: {path.name}')
    mode = path.stat().st_mode
    if not stat.S_ISREG(mode):
        raise ValueError(f'nonregular input refused: {path.name}')
    if max_bytes is not None and path.stat().st_size > max_bytes:
        raise ValueError(f'oversized input: {path.name}')
    return path.read_bytes()


def validate_text(text):
    if re.search(r'(?:/home/|/Users/|[A-Za-z]:\\Users\\|-----BEGIN (?:RSA |EC |OPENSSH )?PRIVATE KEY-----|\b(?:ghp_|github_pat_|sk_live_)[A-Za-z0-9_]{12,})', text):
        raise ValueError('secret-like content or absolute developer path')


def validate_skill(skill):
    observed = set()
    for p in skill.rglob('*'):
        if p.is_symlink():
            raise ValueError('symlink skill member')
        if p.is_file():
            observed.add(p.relative_to(skill).as_posix())
        elif not p.is_dir():
            raise ValueError('nonregular skill member')
    if observed != set(SKILL_FILES):
        raise ValueError(f'skill allowlist mismatch: {sorted(observed)}')
    main = regular_bytes(skill / 'SKILL.md', MAX_SKILL_BYTES).decode('utf-8')
    parts = main.split('---', 2)
    if len(parts) != 3 or parts[0] != '' or not parts[1].startswith('\n'):
        raise ValueError('missing frontmatter')
    metadata = {}
    for line in parts[1].strip().splitlines():
        key, sep, value = line.partition(': ')
        if not sep or key in metadata:
            raise ValueError('invalid frontmatter')
        metadata[key] = value
    if set(metadata) != {'name', 'description'} or metadata['name'] != 'flowspec':
        raise ValueError('invalid skill name/frontmatter fields')
    if len(parts[1]) > 1024 or not metadata['description'].startswith('This skill should be used when'):
        raise ValueError('invalid skill description')
    for name in SKILL_FILES:
        p = skill / name
        text = regular_bytes(p, 64000).decode('utf-8')
        validate_text(text)
        for target in re.findall(r'\]\(([^)]+)\)', text):
            if ':' in target or target.startswith('/'):
                raise ValueError('nonlocal skill link')
            dest = (p.parent / target.split('#')[0]).resolve()
            if not dest.is_relative_to(skill.resolve()) or not dest.is_file():
                raise ValueError('broken or escaping skill link')
    required = ('incomplete', 'declared scope', 'exit 2', 'exit 3', 'trace', 'diff', 'repository', 'stable', 'continuation')
    if not all(word in main.lower() for word in required):
        raise ValueError('incomplete workflow instructions')


def run_json(binary, args, cwd):
    env = {'PATH': os.defpath, 'HOME': str(cwd), 'LC_ALL': 'C', 'TZ': 'UTC'}
    result = subprocess.run([str(binary), *args], cwd=cwd, env=env, capture_output=True, timeout=30)
    if result.returncode != 0:
        raise ValueError(f'binary preflight failed: {args[0]} exit {result.returncode}')
    return json.loads(result.stdout)


def build_package(root, binary, out, expected_sha256):
    root, binary, out = Path(root).absolute(), Path(binary).absolute(), Path(out).absolute()
    validate_skill(root / 'skills/flowspec')
    data = regular_bytes(binary, 256 * 1024 * 1024)
    digest = sha256(data)
    if not re.fullmatch('[a-f0-9]{64}', expected_sha256) or digest != expected_sha256:
        raise ValueError('candidate binary SHA-256 mismatch')
    if not data.startswith(b'\x7fELF') or data[4:6] != bytes((2,1)):
        raise ValueError('candidate is not a Linux ELF64 little-endian binary')
    machine = int.from_bytes(data[18:20], 'little')
    target = {62: 'x86_64-linux', 183: 'aarch64-linux'}.get(machine)
    if target is None:
        raise ValueError('unsupported Linux architecture')
    if not os.access(binary, os.X_OK):
        raise ValueError('candidate binary is not executable')
    with tempfile.TemporaryDirectory(prefix='flowspec-package-') as t:
        sandbox = Path(t)
        (sandbox / 'main.py').write_text('')
        report = run_json(binary, ['analyze', '.', '--format', 'json', '--quiet'], sandbox)
        catalog = run_json(binary, ['registry'], sandbox)
    a = report['authority']
    if a['execution'] != 'complete' or a['outcome'] != 'clean' or a['producer']['generated_by'] != 'analyzer':
        raise ValueError('candidate failed empty-source provenance preflight')
    head = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
    tree = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD^{tree}'], text=True).strip()
    source_paths = ['skills/flowspec', 'scripts/package-skill.py', 'LICENSE', 'LICENSE-AGPL']
    dirty = subprocess.check_output(['git', '-C', str(root), 'status', '--porcelain', '--', *source_paths], text=True)
    if dirty:
        raise ValueError('package source must be committed and clean')
    files = {name: regular_bytes(root / 'skills/flowspec' / name) for name in SKILL_FILES}
    files.update({name: regular_bytes(root / name, 1000000) for name in ('LICENSE','LICENSE-AGPL')})
    files['bin/flowspec'] = data
    manifest = {
        'schema': 'flowspec.skill-package.v1', 'release_qualified': False,
        'source_sha': head, 'source_tree': tree, 'target': target,
        'binary_sha256': digest, 'binary_producer': a['producer'],
        'registry_digest': catalog['digest'],
        'preflight_config_digest': a['provenance']['config']['effective_digest'],
        'members': {name: sha256(value) for name,value in sorted(files.items())},
    }
    encoded = (json.dumps(manifest, sort_keys=True, indent=2)+'\n').encode()
    validate_text(encoded.decode())
    files['package.json'] = encoded
    files['SHA256SUMS'] = ''.join(f'{sha256(value)}  {name}\n' for name,value in sorted(files.items())).encode()
    stream = io.BytesIO()
    with zipfile.ZipFile(stream, 'w', compression=zipfile.ZIP_STORED) as archive:
        for name,value in sorted(files.items()):
            info = zipfile.ZipInfo('flowspec/'+name, (1980,1,1,0,0,0))
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | (0o755 if name=='bin/flowspec' else 0o644)) << 16
            archive.writestr(info, value)
    if sha256(regular_bytes(binary)) != digest:
        raise ValueError('binary changed during packaging')
    out.mkdir(parents=True, exist_ok=True)
    dest = out / f'flowspec-skill-{target}-{digest[:12]}.zip'
    with tempfile.NamedTemporaryFile(dir=out, delete=False) as temp:
        pending = Path(temp.name)
        temp.write(stream.getvalue())
        temp.flush()
        os.fsync(temp.fileno())
    try:
        os.replace(pending, dest)
    finally:
        pending.unlink(missing_ok=True)
    (out / (dest.name+'.sha256')).write_text(f'{sha256(dest.read_bytes())}  {dest.name}\n')
    return dest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--binary-sha256', required=True, help='exact reviewed candidate SHA-256')
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    print('import:', Path(__file__).resolve(), file=sys.stderr)
    try:
        path = build_package(ROOT, args.binary, args.out, args.binary_sha256)
        print(json.dumps({'archive': str(path), 'sha256': sha256(path.read_bytes()), 'release_qualified': False}))
        return 0
    except (ValueError,OSError,KeyError,subprocess.SubprocessError) as exc:
        print(f'HOLD skill package: {exc}', file=sys.stderr)
        return 1

if __name__ == '__main__':
    sys.exit(main())
