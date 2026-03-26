# PLANTED FACTS:
# - typing.TYPE_CHECKING (attribute form) should also be detected
# - Request inside the guard is type-only
import typing

if typing.TYPE_CHECKING:
    from http import Request
