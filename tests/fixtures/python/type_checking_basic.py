# PLANTED FACTS:
# - TYPE_CHECKING imported from typing → NOT phantom
# - PathLike inside TYPE_CHECKING block → type-only import, NOT phantom
# - os imported outside TYPE_CHECKING → regular import, usage rules apply
from typing import TYPE_CHECKING
import os

if TYPE_CHECKING:
    from os import PathLike

def process(path: PathLike) -> None:
    os.path.exists(str(path))
