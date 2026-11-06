"""Fail-closed bounds for recipe agent env knobs, roles, and shell commands."""

from __future__ import annotations

import pathlib
import re

_URL_RE = re.compile(
    r"^https?://([A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?)(?::(\d{1,5}))?(/.*)?$"
)
_MIN_LEASE = 1
_MAX_LEASE = 3600
_MIN_POLL = 0.1
_MAX_POLL = 60.0
_MIN_TIMEOUT = 1
_MAX_TIMEOUT = 7200
_MAX_COMMAND = 4096
_MAX_URL = 2048
_ROLE_RE = re.compile(r"^[a-z][a-z0-9_]{0,31}$")
_SLUG_RE = re.compile(r"^[a-z][a-z0-9_-]{0,63}$")
_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")


def validate_base_url(url: str) -> str:
    if not isinstance(url, str) or not url.strip():
        raise ValueError("MAIDAN_URL is empty")
    url = url.strip()
    if len(url) > _MAX_URL:
        raise ValueError("MAIDAN_URL is too long")
    if any(ch.isspace() for ch in url):
        raise ValueError("MAIDAN_URL must not contain whitespace")
    hostport = url.split("://", 1)[-1].split("/", 1)[0]
    if "@" in hostport:
        raise ValueError("MAIDAN_URL must not embed credentials")
    if ".." in url:
        raise ValueError("MAIDAN_URL must not contain ..")
    match = _URL_RE.match(url)
    if not match:
        raise ValueError("MAIDAN_URL must be http(s)://host[:port][/path]")
    port = match.group(2)
    if port is not None and not (1 <= int(port) <= 65535):
        raise ValueError("MAIDAN_URL port out of range")
    return url.rstrip("/")


def validate_lease_secs(value: object) -> int:
    try:
        if isinstance(value, bool):
            raise ValueError("LEASE_SECS must be an int")
        lease = int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError) as exc:
        raise ValueError("LEASE_SECS must be an int") from exc
    if lease < _MIN_LEASE or lease > _MAX_LEASE:
        raise ValueError(f"LEASE_SECS must be {_MIN_LEASE}..{_MAX_LEASE}")
    return lease


def validate_poll_secs(value: object) -> float:
    try:
        poll = float(value)  # type: ignore[arg-type]
    except (TypeError, ValueError) as exc:
        raise ValueError("POLL_SECS must be a number") from exc
    if poll < _MIN_POLL or poll > _MAX_POLL:
        raise ValueError(f"POLL_SECS must be {_MIN_POLL}..{_MAX_POLL}")
    return poll


def validate_run_timeout_secs(value: object) -> int:
    try:
        if isinstance(value, bool):
            raise ValueError("RUN_TIMEOUT_SECS must be an int")
        timeout = int(value)  # type: ignore[arg-type]
    except (TypeError, ValueError) as exc:
        raise ValueError("RUN_TIMEOUT_SECS must be an int") from exc
    if timeout < _MIN_TIMEOUT or timeout > _MAX_TIMEOUT:
        raise ValueError(f"RUN_TIMEOUT_SECS must be {_MIN_TIMEOUT}..{_MAX_TIMEOUT}")
    return timeout


def validate_shell_command(command: str, label: str) -> str:
    """Allow empty (stand-in path); otherwise bound length and refuse NUL/newlines."""
    if not isinstance(command, str):
        raise ValueError(f"{label} must be a string")
    command = command.strip()
    if not command:
        return ""
    if len(command) > _MAX_COMMAND:
        raise ValueError(f"{label} is too long")
    if "\0" in command or "\n" in command or "\r" in command:
        raise ValueError(f"{label} must not contain control characters")
    if "CHANGE_ME" in command:
        raise ValueError(f"{label} still holds a placeholder")
    return command


def validate_role(role: str, allowed: set[str] | frozenset[str] | dict) -> str:
    if role not in allowed:
        raise ValueError(f"role must be one of: {', '.join(sorted(allowed))}")
    if not _ROLE_RE.match(role):
        raise ValueError("role has invalid characters")
    return role


def validate_channel_name(name: str) -> str:
    if not isinstance(name, str) or not _SLUG_RE.match(name):
        raise ValueError("channel name is invalid")
    return name


def validate_id(value: str, label: str) -> str:
    if not isinstance(value, str) or not _ID_RE.match(value):
        raise ValueError(f"{label} is invalid")
    return value


def validate_init_path(path: str | pathlib.Path) -> pathlib.Path:
    candidate = pathlib.Path(path)
    if ".." in candidate.parts:
        raise ValueError("init path must not traverse")
    if not candidate.is_file():
        raise ValueError("init path is not a file")
    return candidate
