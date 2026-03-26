# PLANTED FACTS:
# - `if not TYPE_CHECKING:` is NOT a type-checking guard
# - Imports inside should be treated as REGULAR imports
from typing import TYPE_CHECKING

if not TYPE_CHECKING:
    from os import getcwd
