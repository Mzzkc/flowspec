# PLANTED FACTS:
# - TYPE_CHECKING = False is an assignment, NOT a guard block
# - This should NOT trigger type-checking annotation
# - The import below should follow normal rules
TYPE_CHECKING = False

if TYPE_CHECKING:
    from os import path
