"""Final package workflow contract. A HOLD must remain a failing qualification."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
class WorkflowTests(unittest.TestCase):
    def test_packaged_ordinary_workflow_has_no_holds(self):
        runner=ROOT/'tests/skill/run-workflow.py'
        self.assertTrue(runner.is_file(), 'packaged ordinary workflow runner is absent')
        archive=os.environ.get('FLOWSPEC_SKILL_ARCHIVE')
        self.assertIsNotNone(archive, 'set exact source-bound FLOWSPEC_SKILL_ARCHIVE')
        with tempfile.TemporaryDirectory() as out:
            proc=subprocess.run(['python3',str(runner),'--archive',archive,'--out',out],capture_output=True,text=True,timeout=120)
            result=json.loads((Path(out)/'workflow.json').read_text())
            self.assertEqual(proc.returncode,0,json.dumps(result['holds']))
            self.assertEqual(result['holds'],[])
            self.assertEqual(set(result['earned']),{'clean_exit_0','error_exit_1','findings_exit_2','incomplete_exit_3','diagnose_exact','trace_exact','comparable_diff','incomparable_diff','sarif_incomplete','continuation','native_validation_handoff'})
if __name__=='__main__':unittest.main()
