import importlib.util
import json
import os
from pathlib import Path
import tempfile
import unittest
ROOT=Path(__file__).resolve().parents[2]
class PublicContractTests(unittest.TestCase):
    def test_generated_tables_reject_drift(self):
        script=ROOT/'scripts/check-product-contract.py'
        self.assertTrue(script.is_file(),'live registry documentation checker absent')
        spec=importlib.util.spec_from_file_location('candidate_public_contract',script)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        print('import:',module.__file__)
        binary=os.environ.get('FLOWSPEC_SKILL_BINARY')
        self.assertIsNotNone(binary,'bind exact binary for documentation agreement')
        catalog=module.read_catalog(Path(binary))
        with tempfile.TemporaryDirectory() as t:
            root=Path(t);(root/'docs').mkdir()
            for path,text in module.render_tables(catalog).items():(root/path).write_text(text)
            module.check_tables(root,catalog)
            p=root/'docs/diagnostics.md';p.write_text(p.read_text().replace('data_dead_end','dead_end_guess'))
            with self.assertRaises(ValueError):module.check_tables(root,catalog)
    def test_unsealed_corpus_is_not_release_agreement(self):
        script=ROOT/'scripts/check-product-contract.py'
        self.assertTrue(script.is_file(),'live public agreement checker absent')
        spec=importlib.util.spec_from_file_location('candidate_public',script)
        module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as t:
            with self.assertRaises(ValueError):module.require_corpus(Path(t))
if __name__=='__main__':unittest.main()
