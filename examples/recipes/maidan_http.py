"""A dependency-free Maidan client for the compose recipes: REST plus stateless MCP.

The recipes run in a stock `python:3.13-slim` container, so this uses only the
standard library. For anything longer-lived, use the SDK in `sdk/python`.
"""

from __future__ import annotations

import json
import re
import os
import pathlib
import threading
import urllib.error
import urllib.request
from urllib.parse import unquote
from typing import Any

MCP_PROTOCOL_VERSION = "2026-07-28"
CLIENT_INFO = {"name": "maidan-recipes", "version": "1"}


_URL_RE = re.compile(
    r"^https?://([A-Za-z0-9](?:[A-Za-z0-9.-]*[A-Za-z0-9])?)(?::(\d{1,5}))?(/.*)?$"
)
_TOKEN_RE = re.compile(r"^[A-Za-z0-9._~+/-]+=*$")
_PATH_RE = re.compile(r"^/[A-Za-z0-9._~!$&'()*+,;=:@%/-]*$")
_TOOL_RE = re.compile(r"^[A-Za-z][A-Za-z0-9_]{0,127}$")
_ID_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")
_MAX_TOKEN = 8192
_MAX_URL = 2048
_MAX_PATH = 1024
_MIN_LEASE = 1
_MAX_LEASE = 3600
_MIN_TIMEOUT = 1.0
_MAX_TIMEOUT = 120.0


def validate_base_url(url: str) -> str:
    """Refuse empty, oversized, credentialed, or non-http(s) recipe origins."""
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


def validate_token(token: str) -> str:
    """Refuse empty, placeholder, whitespace, or oversized bearer tokens."""
    if not isinstance(token, str) or not token:
        raise ValueError("token is empty")
    if len(token) > _MAX_TOKEN:
        raise ValueError("token is too long")
    if any(ch.isspace() for ch in token):
        raise ValueError("token must not contain whitespace")
    if "CHANGE_ME" in token or token in {"changeme", "secret", "password"}:
        raise ValueError("token still holds a placeholder")
    if not _TOKEN_RE.match(token):
        raise ValueError("token has invalid characters")
    return token


def validate_rest_path(path: str) -> str:
    """Refuse relative, traversal, or oversized REST paths."""
    if not isinstance(path, str) or not path:
        raise ValueError("path is empty")
    if len(path) > _MAX_PATH:
        raise ValueError("path is too long")
    if not path.startswith("/") or path.startswith("//"):
        raise ValueError("path must be a single absolute REST path")
    if "\\" in path or chr(0) in path:
        raise ValueError("path must not traverse or escape")
    if not _PATH_RE.match(path):
        raise ValueError("path has invalid characters")
    decoded = unquote(path)
    if ".." in path or ".." in decoded or "\\" in decoded:
        raise ValueError("path must not traverse or escape")
    if not decoded.startswith("/") or decoded.startswith("//"):
        raise ValueError("path must be a single absolute REST path")
    return path


def validate_lease_secs(lease_secs: int) -> int:
    if not isinstance(lease_secs, int) or isinstance(lease_secs, bool):
        raise ValueError("lease_secs must be an int")
    if lease_secs < _MIN_LEASE or lease_secs > _MAX_LEASE:
        raise ValueError(f"lease_secs must be {_MIN_LEASE}..{_MAX_LEASE}")
    return lease_secs


def validate_timeout(timeout: float) -> float:
    try:
        value = float(timeout)
    except (TypeError, ValueError) as exc:
        raise ValueError("timeout must be a number") from exc
    if value < _MIN_TIMEOUT or value > _MAX_TIMEOUT:
        raise ValueError(f"timeout must be {_MIN_TIMEOUT}..{_MAX_TIMEOUT}")
    return value


def validate_tool_name(name: str) -> str:
    if not isinstance(name, str) or not _TOOL_RE.match(name):
        raise ValueError("MCP tool name is invalid")
    return name


def validate_id(value: str, label: str) -> str:
    if not isinstance(value, str) or not _ID_RE.match(value):
        raise ValueError(f"{label} is invalid")
    return value


def validate_creds_path(path: str) -> pathlib.Path:
    if not isinstance(path, str) or not path.strip():
        raise ValueError("creds path is empty")
    if len(path) > _MAX_PATH:
        raise ValueError("creds path is too long")
    if chr(0) in path or "\n" in path or "\r" in path:
        raise ValueError("creds path has invalid characters")
    candidate = pathlib.Path(path)
    if ".." in candidate.parts:
        raise ValueError("creds path must not traverse")
    if not candidate.is_file():
        raise ValueError("creds path is not a file")
    return candidate



class MaidanError(RuntimeError):
    def __init__(self, method: str, path: str, status: int, detail: str) -> None:
        super().__init__(f"{method} {path}: HTTP {status}: {detail}")
        self.status = status


class Maidan:
    def __init__(self, base_url: str, token: str, *, timeout: float = 60.0) -> None:
        self.base_url = validate_base_url(base_url)
        self.token = validate_token(token)
        self.timeout = validate_timeout(timeout)
        self._mcp_id = 0

    def call(
        self,
        method: str,
        path: str,
        body: Any = None,
        *,
        raw: bytes | None = None,
        content_type: str = "application/json",
        headers: dict[str, str] | None = None,
    ) -> Any:
        data = raw if raw is not None else None
        if body is not None:
            data = json.dumps(body).encode("utf-8")
        path = validate_rest_path(path)
        request = urllib.request.Request(
            f"{self.base_url}{path}",
            data=data,
            method=method,
            headers={
                "authorization": f"Bearer {self.token}",
                "accept": "application/json",
                "content-type": content_type,
                **(headers or {}),
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=self.timeout) as response:
                payload = response.read()
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise MaidanError(method, path, error.code, detail) from error
        return json.loads(payload) if payload else None

    def tool(self, name: str, arguments: dict[str, Any]) -> Any:
        """Call one MCP tool on `2026-07-28`: stateless, no initialize, no session.

        That revision states itself on every request, and the server refuses a
        request that leaves any of it out (docs/Integration Reference.md, "MCP
        streamable"): the `MCP-Protocol-Version` header, the same version in
        `params._meta` beside the client's capabilities, and `Mcp-Method` and
        `Mcp-Name` headers naming the method and the tool.
        """
        name = validate_tool_name(name)
        self._mcp_id += 1
        method = "tools/call"
        reply = self.call(
            "POST",
            "/mcp/streamable",
            {
                "jsonrpc": "2.0",
                "id": self._mcp_id,
                "method": method,
                "params": {
                    "name": name,
                    "arguments": arguments,
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": MCP_PROTOCOL_VERSION,
                        "io.modelcontextprotocol/clientCapabilities": {},
                        "io.modelcontextprotocol/clientInfo": CLIENT_INFO,
                    },
                },
            },
            headers={
                "mcp-protocol-version": MCP_PROTOCOL_VERSION,
                "mcp-method": method,
                "mcp-name": name,
            },
        )
        if "error" in reply:
            raise RuntimeError(f"{name}: {reply['error']}")
        result = reply["result"]
        text = result["content"][0]["text"]
        if result.get("isError"):
            raise RuntimeError(f"{name}: {text}")
        return json.loads(text)


class Claim:
    """One claimed task: acknowledged on entry, renewed while held, released on exit.

    A task claimed without a lease that is then never released stays assigned
    to a dead agent, so the lease is kept alive from a background thread for as
    long as the work takes, and released however the work ends. A failed
    renewal is remembered (`renewal_error`) so the caller can stop; if release
    is then refused, the lease is left to lapse.
    """

    def __init__(self, maidan: Maidan, thread: dict[str, Any], lease_secs: int) -> None:
        self.maidan = maidan
        self.thread_id = thread["id"]
        self.lease_id = thread["claim_lease_id"]
        self.lease_secs = validate_lease_secs(lease_secs)
        validate_id(self.thread_id, "thread_id")
        validate_id(self.lease_id, "claim_lease_id")
        self._stop = threading.Event()
        self._renewer = threading.Thread(target=self._renew, daemon=True)
        self._lock = threading.Lock()
        # Set from the renewer thread. The caller reads it and stops work.
        self._renewal_error: str | None = None

    def __enter__(self) -> "Claim":
        self.maidan.call(
            "POST",
            f"/threads/{self.thread_id}/claim/acknowledge",
            {"claim_lease_id": self.lease_id},
        )
        self._renewer.start()
        return self

    def renewal_error(self) -> str | None:
        """Why the last renewal failed, if one has. `None` means the lease is held."""
        with self._lock:
            return self._renewal_error

    def _renew(self) -> None:
        while not self._stop.wait(self.lease_secs / 3):
            try:
                self.maidan.call(
                    "POST",
                    f"/threads/{self.thread_id}/claim/renew",
                    {"claim_lease_id": self.lease_id, "lease_secs": self.lease_secs},
                )
            except MaidanError as error:
                # One failure is enough: the caller must stop. The lease then
                # either releases on the way out, or lapses if release is refused.
                with self._lock:
                    self._renewal_error = str(error)
                return

    def __exit__(self, *_exc: object) -> None:
        self._stop.set()
        self._renewer.join()
        try:
            self.maidan.call(
                "POST",
                f"/threads/{self.thread_id}/claim/release",
                {"claim_lease_id": self.lease_id},
            )
        except MaidanError:
            # Release was refused, usually because the lease is already gone.
            # Letting it lapse is the other ending the recipes allow. A renewal
            # failure is reported by the caller; a release failure on its own
            # still propagates.
            if self.renewal_error() is None:
                raise

    def task(self) -> str:
        """The thread's opening message: what was asked.

        A thread is claimable the moment it exists, which can be before its
        first message is posted; the title stands in until then.
        """
        context = self.maidan.call("GET", f"/threads/{self.thread_id}/context")
        if context.get("messages"):
            return context["messages"][0]["body"]
        return self.maidan.call("GET", f"/threads/{self.thread_id}")["title"] or ""

    def post(self, body: str, artifacts: list[str] | None = None) -> None:
        message: dict[str, Any] = {"body": body}
        if artifacts:
            message["metadata"] = {"artifacts": artifacts}
        self.maidan.call("POST", f"/threads/{self.thread_id}/messages", message)

    def finish(self, result: dict[str, Any]) -> None:
        """Hand the answer back, then the thread to review.

        Only `open` threads are handed out, so this is what keeps the release
        that follows from putting finished work back in the queue. Closing it
        is someone else's call.
        """
        self.maidan.call(
            "PUT",
            f"/threads/{self.thread_id}/result",
            {"claim_lease_id": self.lease_id, "result": result},
        )
        self.maidan.call("POST", f"/threads/{self.thread_id}", {"action": "start_review"})


def claim_next(maidan: Maidan, channel_id: str, lease_secs: int) -> Claim | None:
    """`None` is the ordinary answer on an idle channel, not an error."""
    channel_id = validate_id(channel_id, "channel_id")
    lease_secs = validate_lease_secs(lease_secs)
    thread = maidan.call(
        "POST", f"/channels/{channel_id}/threads/claim-next", {"lease_secs": lease_secs}
    )
    return Claim(maidan, thread, lease_secs) if thread else None


def from_creds(path: str = "/creds/agent.json") -> tuple[Maidan, dict[str, str]]:
    """The scoped identity `provision.py` minted for this agent."""
    creds_path = validate_creds_path(path)
    creds = json.loads(creds_path.read_text())
    if not isinstance(creds, dict):
        raise ValueError("creds file must be a JSON object")
    url = os.environ.get("MAIDAN_URL", creds.get("url", ""))
    token = creds.get("token", "")
    return Maidan(url, token), creds
