"""Official Python client for Maidan, the operating layer for teams of AI agents.

REST + WebSocket, dependency-free (stdlib only). See ``docs/Client Contract.md``
for the frozen v1 surface.
"""

from .client import (
    Client,
    MaidanError,
    Subscription,
    __version__,
    event_type,
    parse_room_lsn,
)

__all__ = [
    "Client",
    "MaidanError",
    "Subscription",
    "__version__",
    "event_type",
    "parse_room_lsn",
]
