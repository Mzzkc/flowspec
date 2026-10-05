"""Distribution boundary tests; all hostile controls run the real packager."""
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import tempfile
import unittest
import zipfile

ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / 'scripts/package-skill.py'

class PackageTests(unittest.TestCase):
    def setUp(self):
        self.assertTrue(SCRIPT.is_file(), 'canonical executable skill packager is absent')
        spec = importlib.util.spec_from_file_location('candidate_package', SCRIPT)
        self.package = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.package)
        print('import:', self.package.__file__)
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name) / 'source'
        self.root.mkdir()
        for name in ['skills/flowspec', 'LICENSE', 'LICENSE-AGPL']:
            src, dst = ROOT / name, self.root / name
            self.assertTrue(src.exists(), f'required package source missing: {name}')
            if src.is_dir():
                shutil.copytree(src, dst)
            else:
                shutil.copy2(src, dst)

    def test_skill_frontmatter_and_contract(self):
        self.package.validate_skill(self.root / 'skills/flowspec')
        body = (self.root / 'skills/flowspec/SKILL.md').read_text()
        self.assertLessEqual(len(body.encode()), 12000)
        for term in ['incomplete', 'declared scope', 'exit 2', 'exit 3', 'trace', 'diff', 'repository', 'stable', 'continuation']:
            self.assertIn(term, body.lower())

    def test_reject_invalid_frontmatter_links_size_secret_and_path(self):
        main = self.root / 'skills/flowspec/SKILL.md'
        original = main.read_text()
        for invalid in [original.replace('name: flowspec', 'name: other'), original.replace('description:', 'unknown:'), original+'\n[escape](../../LICENSE)', original+'\n[missing](references/absent.md)', original+'x'*12001, original+'\n/home/developer/private', original+'\n-----BEGIN PRIVATE KEY-----']:
            with self.subTest(control=invalid[-60:]):
                main.write_text(invalid)
                with self.assertRaises(ValueError):
                    self.package.validate_skill(main.parent)
        main.write_text(original)

    def test_reject_extra_file_and_symlink(self):
        extra = self.root / 'skills/flowspec/.env'
        extra.write_text('secret=private')
        with self.assertRaises(ValueError):
            self.package.validate_skill(extra.parent)
        extra.unlink()
        extra.symlink_to(self.root / 'LICENSE')
        with self.assertRaises(ValueError):
            self.package.validate_skill(extra.parent)

    def test_package_binary_digest_license_and_determinism(self):
        binary = Path(os.environ.get('FLOWSPEC_SKILL_BINARY', '/missing-candidate'))
        self.assertTrue(binary.is_file(), 'set FLOWSPEC_SKILL_BINARY to an immutable candidate')
        digest = hashlib.sha256(binary.read_bytes()).hexdigest()
        out = Path(self.tmp.name) / 'out'
        first = self.package.build_package(ROOT, binary, out, digest)
        first_bytes = first.read_bytes()
        second = self.package.build_package(ROOT, binary, out, digest)
        self.assertEqual(first_bytes, second.read_bytes())
        with zipfile.ZipFile(first) as z:
            self.assertEqual(set(z.namelist()), {'flowspec/SKILL.md', 'flowspec/references/report-contract.md', 'flowspec/references/examples.md', 'flowspec/LICENSE', 'flowspec/LICENSE-AGPL', 'flowspec/bin/flowspec', 'flowspec/package.json', 'flowspec/SHA256SUMS'})
            self.assertEqual(hashlib.sha256(z.read('flowspec/bin/flowspec')).hexdigest(), digest)
            self.assertEqual(z.read('flowspec/LICENSE'), (ROOT/'LICENSE').read_bytes())
            self.assertEqual(z.read('flowspec/LICENSE-AGPL'), (ROOT/'LICENSE-AGPL').read_bytes())
            manifest = json.loads(z.read('flowspec/package.json'))
            self.assertEqual(manifest['binary_sha256'], digest)
            self.assertFalse(manifest['release_qualified'])
            for row in z.read('flowspec/SHA256SUMS').decode().splitlines():
                sha, name = row.split('  ')
                self.assertEqual(hashlib.sha256(z.read('flowspec/'+name)).hexdigest(), sha)
        with self.assertRaises(ValueError):
            self.package.build_package(ROOT, binary, out, '0'*64)

if __name__ == '__main__':
    unittest.main()
