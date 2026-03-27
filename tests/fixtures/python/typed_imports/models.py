from typing import Optional

class User:
    name: str = "anonymous"
    email: Optional[str] = None

    def get_name(self) -> str:
        return self.name
