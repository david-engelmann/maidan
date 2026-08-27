"""Client for Maidan. 0.0.1 is a name reservation; the API is not stable."""

__version__ = "0.0.1"


class Client:
    def __init__(self, base_url: str, token: str) -> None:
        self.base_url = base_url
        self.token = token
