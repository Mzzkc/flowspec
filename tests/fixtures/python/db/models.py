"""Data layer models and queries."""


def get_user(user_id):
    """Fetch user from database."""
    return {"id": user_id, "name": "test"}
