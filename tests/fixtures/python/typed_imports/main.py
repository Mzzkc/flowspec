from typing import Optional, Dict, Union, List

def greet(name: Optional[str]) -> str:
    return name or "world"

def process(data: Dict[str, int], flags: Union[str, int]) -> List[str]:
    return list(data.keys())

x: int = 42
