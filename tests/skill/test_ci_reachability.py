import importlib.util
from pathlib import Path
import unittest
ROOT=Path(__file__).resolve().parents[2]
spec=importlib.util.spec_from_file_location('product_ci_reachability',ROOT/'scripts/check-product-contract.py')
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
class Reachability(unittest.TestCase):
 def test_early_success_cannot_hide_unreachable_commands(self):
  original=(ROOT/'.github/workflows/ci.yml').read_text()
  bypass=original.replace('          cargo +1.91.0 fmt --all --check','          exit 0\n          cargo +1.91.0 fmt --all --check')
  self.assertNotEqual(original,bypass)
  with self.assertRaises(ValueError):module.validate_ci(bypass)
