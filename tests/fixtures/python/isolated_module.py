# PLANTED FACTS:
# - Processor, process(), validate() form an isolated cluster of symbols.
# - They reference each other internally but NOTHING outside this file calls into them.
# - TRUE POSITIVE for isolated_cluster (WARNING severity, HIGH confidence).

class Processor:
    """Internal class — never instantiated outside this module."""
    def __init__(self, config):
        self.config = config

    def run(self, data):
        """Calls process() — internal reference."""
        return process(data, self.config)

def process(data, config):
    """Calls validate() — internal reference."""
    if validate(data):
        return data.upper()
    return None

def validate(data):
    """Leaf function in the cluster. Called only by process()."""
    return data is not None and len(data) > 0
