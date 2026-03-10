# PLANTED FACTS:
# - `import logging` used for side-effect (logging.basicConfig).
#   The module `logging` IS referenced (prefix usage). TRUE NEGATIVE for phantom_dependency.
# - `import json` — json is truly never referenced. TRUE POSITIVE for phantom_dependency.

import logging
import json

logging.basicConfig(level=logging.DEBUG)

def do_work():
    return "work done"
