"""API route handlers."""

from db.models import get_user


def handle_request(user_id):
    """Handle incoming API request."""
    user = get_user(user_id)
    return user
