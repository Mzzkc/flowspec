"""Mutate real workflow documents and require the contract checker to refuse."""
import importlib.util
from pathlib import Path
import unittest

ROOT = Path(__file__).resolve().parents[2]
class CIContractTests(unittest.TestCase):
    def setUp(self):
        script = ROOT / 'scripts/check-product-contract.py'
        self.assertTrue(script.is_file(), 'executable CI/public contract checker is absent')
        spec = importlib.util.spec_from_file_location('candidate_contract', script)
        self.module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.module)
        print('import:', self.module.__file__)
        self.text = (ROOT / '.github/workflows/ci.yml').read_text()

    def test_actual_workflow(self):
        self.module.validate_ci(self.text)

    def test_reject_moving_pin_permissions_soft_pass_and_partial_commands(self):
        changes = [('permissions:\n  contents: read', 'permissions:\n  contents: write'), ('--workspace --all-targets --all-features', '-p flowspec --lib'), ('--locked --workspace --all-features', '--workspace'), ('@3d3c42e5aac5ba805825da76410c181273ba90b1', '@v7'), ('rustup toolchain install 1.91.0', 'rustup toolchain install stable'), ('python-version: 3.11.15', 'python-version: 3.x')]
        for old, new in changes:
            self.assertIn(old, self.text)
            with self.subTest(mutation=new), self.assertRaises(ValueError):
                self.module.validate_ci(self.text.replace(old,new))
        for suffix in ['\n# continue-on-error: true\n', '\n# pull_request_target:\n']:
            with self.assertRaises(ValueError):
                self.module.validate_ci(self.text+suffix)

    def test_missing_gates_are_rejected(self):
        for gate in ['coverage', 'oracle', 'security', 'reproducibility', 'cache-equivalence', 'test-inventory']:
            with self.subTest(gate=gate), self.assertRaises(ValueError):
                self.module.validate_ci(self.text.replace('check-ci-prerequisites.py '+gate, 'echo omitted'))

    def test_custody_marker_command_reaches_live_checker(self):
        import subprocess, shlex
        marker_commands=[line.strip() for line in self.text.splitlines() if line.strip().startswith('python3 scripts/check-test-custody.py') and '--check-markers' in line]
        self.assertEqual(len(marker_commands),1)
        command=shlex.split(marker_commands[0].split(' 2>&1')[0])
        result=subprocess.run(command,cwd=ROOT,capture_output=True,text=True,timeout=30)
        self.assertEqual(result.returncode,0,result.stderr)

    def test_missing_prerequisites_fail_closed(self):
        import subprocess,tempfile,json
        with tempfile.TemporaryDirectory() as t:
            r=subprocess.run(['python3',str(ROOT/'scripts/check-ci-prerequisites.py'),'coverage','--root',t,'--out',t],capture_output=True,text=True)
            self.assertEqual(r.returncode, 1)
            result=json.loads((Path(t)/'coverage-prerequisite.json').read_text())
            self.assertEqual(result['status'], 'HOLD')
            self.assertIn('scripts/coverage.sh',result['missing'])

if __name__ == '__main__':
    unittest.main()
