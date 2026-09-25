"""A dependency-free Maidan client for the compose recipes: REST plus stateless MCP.

The recipes run in a stock `python:3.13-slim` container, so this uses only the
standard library. For anything longer-lived, use the SDK in `sdk/python`.
"""

from __future__ import annotations

import json
import os
import pathlib
import threading
import urllib.error
import urllib.request
from typing import Any

MCP_PROTOCOL_VERSION = "2026-07-28"


class MaidanError(RuntimeError):
    def __init__(self, method: str, path: str, status: int, detail: str) -> None:
        super().__init__(f"{method} {path}: HTTP {status}: {detail}")
        self.status = status


class Maidan:
    def __init__(self, base_url: str, token: str) -> None:
        self.base_url = base_url.rstrip("/")
        self.token = token
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
            with urllib.request.urlopen(request, timeout=60) as response:
                payload = response.read()
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            raise MaidanError(method, path, error.code, detail) from error
        return json.loads(payload) if payload else None

    def tool(self, name: str, arguments: dict[str, Any]) -> Any:
        """Call one MCP tool. `2026-07-28` is stateless: no initialize, no session."""
        self._mcp_id += 1
        reply = self.call(
            "POST",
            "/mcp/streamable",
            {
                "jsonrpc": "2.0",
                "id": self._mcp_id,
                "method": "tools/call",
                "params": {"name": name, "arguments": arguments},
            },
            headers={"mcp-protocol-version": MCP_PROTOCOL_VERSION},
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
    long as the work takes, and released however the work ends.
    """

    def __init__(self, maidan: Maidan, thread: dict[str, Any], lease_secs: int) -> None:
        self.maidan = maidan
        self.thread_id = thread["id"]
        self.lease_id = thread["claim_lease_id"]
        self.lease_secs = lease_secs
        self._stop = threading.Event()
        self._renewer = threading.Thread(target=self._renew, daemon=True)

    def __enter__(self) -> "Claim":
        self.maidan.call(
            "POST",
            f"/threads/{self.thread_id}/claim/acknowledge",
            {"claim_lease_id": self.lease_id},
        )
        self._renewer.start()
        return self

    def _renew(self) -> None:
        while not self._stop.wait(self.lease_secs / 3):
            self.maidan.call(
                "POST",
                f"/threads/{self.thread_id}/claim/renew",
                {"claim_lease_id": self.lease_id, "lease_secs": self.lease_secs},
            )

    def __exit__(self, *_exc: object) -> None:
        self._stop.set()
        self._renewer.join()
        self.maidan.call(
            "POST",
            f"/threads/{self.thread_id}/claim/release",
            {"claim_lease_id": self.lease_id},
        )

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
        self.maidan.call("PUT", f"/threads/{self.thread_id}/result", {"result": result})
        self.maidan.call("POST", f"/threads/{self.thread_id}", {"action": "start_review"})


def claim_next(maidan: Maidan, channel_id: str, lease_secs: int) -> Claim | None:
    """`None` is the ordinary answer on an idle channel, not an error."""
    thread = maidan.call(
        "POST", f"/channels/{channel_id}/threads/claim-next", {"lease_secs": lease_secs}
    )
    return Claim(maidan, thread, lease_secs) if thread else None


def from_creds(path: str = "/creds/agent.json") -> tuple[Maidan, dict[str, str]]:
    """The scoped identity `provision.py` minted for this agent."""
    creds = json.loads(pathlib.Path(path).read_text())
    url = os.environ.get("MAIDAN_URL", creds["url"])
    return Maidan(url, creds["token"]), creds
