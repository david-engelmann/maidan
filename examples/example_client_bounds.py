"""Fail-closed URL/token/channel/lease bounds shared by example MCP clients."""

from __future__ import annotations

import re
from urllib.parse import unquote, urlsplit

_URL_RE = re.compile(
    r"^https?://([A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?)(?::(\d{1,5}))?(/.*)?$"
)
_TOKEN_RE = re.compile(r"^[A-Za-z0-9._~+/-]+=*$")
_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_MAX_URL = 2048
_MAX_TOKEN = 8192
_MIN_LEASE = 1
_MAX_LEASE = 3600
_LOOPBACK = frozenset({"127.0.0.1", "localhost", "::1"})


def validate_base_url(url: str, *, allow_plain_http_nonlocal: bool = False) -> str:
    """Refuse empty, credentialed, or non-http(s) origins.

    Plain http to a non-loopback host is refused unless allow_plain_http_nonlocal
    is set (compose recipes that talk to a service name on a private network).
    """
    if not isinstance(url, str) or not url.strip():
        raise ValueError("MAIDAN_URL is empty")
    url = url.strip().rstrip("/")
    if len(url) > _MAX_URL:
        raise ValueError("MAIDAN_URL is too long")
    if any(ch.isspace() for ch in url):
        raise ValueError("MAIDAN_URL must not contain whitespace")
    parts = urlsplit(url)
    if parts.scheme not in ("http", "https"):
        raise ValueError("MAIDAN_URL must be http(s)")
    if parts.username or parts.password:
        raise ValueError("MAIDAN_URL must not embed credentials")
    if ".." in url or ".." in unquote(url):
        raise ValueError("MAIDAN_URL must not contain ..")
    if not _URL_RE.match(url):
        raise ValueError("MAIDAN_URL must be http(s)://host[:port][/path]")
    if parts.port is not None and not (1 <= parts.port <= 65535):
        raise ValueError("MAIDAN_URL port out of range")
    host = (parts.hostname or "").lower()
    if parts.scheme == "http" and host not in _LOOPBACK and not allow_plain_http_nonlocal:
        raise ValueError(
            f"refusing to send the token to {url} over plain http; use https or a loopback address"
        )
    return url


def validate_token(token: str) -> str:
    if not isinstance(token, str) or not token:
        raise ValueError("MAIDAN_TOKEN is empty")
    if len(token) > _MAX_TOKEN:
        raise ValueError("MAIDAN_TOKEN is too long")
    if any(ch.isspace() for ch in token):
        raise ValueError("MAIDAN_TOKEN must not contain whitespace")
    if "CHANGE_ME" in token or token in {"changeme", "secret", "password"}:
        raise ValueError("MAIDAN_TOKEN still holds a placeholder")
    if not _TOKEN_RE.match(token):
        raise ValueError("MAIDAN_TOKEN has invalid characters")
    return token


def validate_channel_id(channel: str) -> str:
    if not isinstance(channel, str) or not _ID_RE.match(channel):
        raise ValueError("channel id is invalid")
    if ".." in channel or ".." in unquote(channel):
        raise ValueError("channel id must not traverse")
    return channel


def validate_lease_secs(value: object) -> int:
    try:
        if isinstance(value, bool):
            raise ValueError("lease_secs must be an int")
        lease = int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError) as exc:
        raise ValueError("lease_secs must be an int") from exc
    if lease < _MIN_LEASE or lease > _MAX_LEASE:
        raise ValueError(f"lease_secs must be {_MIN_LEASE}..{_MAX_LEASE}")
    return lease
