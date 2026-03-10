# PLANTED FACTS:
# - `import os`: os is NEVER referenced anywhere. TRUE POSITIVE for phantom_dependency (HIGH confidence).
# - `from collections import OrderedDict`: OrderedDict never used. TRUE POSITIVE for phantom_dependency.
# - `from pathlib import Path`: Path IS used on line ~15. TRUE NEGATIVE for phantom_dependency.
# - `import sys`: sys.argv used on line ~12. TRUE NEGATIVE (prefix usage — sys IS referenced).
# - `from typing import Optional`: Optional used only in type annotation. TRUE NEGATIVE.

import os
from collections import OrderedDict
from pathlib import Path
import sys
from typing import Optional

def get_args():
    """Uses sys.argv — sys import is NOT phantom."""
    return sys.argv[1:]

def resolve_path(name: str) -> Optional[Path]:
    """Uses Path AND Optional — both imports are valid."""
    p = Path(name)
    if p.exists():
        return p
    return None

def process():
    """Does NOT use os or OrderedDict."""
    return "done"
