"""Maidan Python client (v1 surface). REST + WebSocket, dependency-free (stdlib
only). See docs/Client Contract.md for the frozen surface.

REST rides ``urllib``; the WebSocket subscribe rides a small stdlib RFC-6455
client (``_WebSocketConn``) so the package needs no third-party dependency.
"""

from __future__ import annotations

import base64
import json
import os
import socket
import ssl
import struct
import threading
import urllib.error
import urllib.parse
import urllib.request
from typing import Any, Callable, Optional

__version__ = "0.1.0"

__all__ = [
    "Client",
    "MaidanError",
    "Subscription",
    "__version__",
    "event_type",
    "parse_room_lsn",
]


def parse_room_lsn(value: Optional[str]) -> Optional[int]:
    """Parse ``Maidan-Room-LSN``. Rejects WAL text so this is never a
    ``Maidan-Consistency-Token``.
    """
    if not value:
        return None
    trimmed = value.strip()
    if not trimmed or "/" in trimmed:
        return None
    try:
        n = int(trimmed, 10)
    except ValueError:
        return None
    if n < 0:
        return None
    return n


def event_type(kind: str) -> str:
    """Observable ``$type`` for an event kind (``message_posted`` → ``maidan.event.message_posted/1``)."""
    return f"maidan.event.{kind}/1"


class MaidanError(Exception):
    """A failed request. Carries the HTTP status and the server's parsed body."""

    def __init__(self, status: int, body: Any, message: Optional[str] = None):
        super().__init__(message or f"Maidan request failed: HTTP {status}")
        self.status = status
        self.body = body
        # Seconds from ``Retry-After`` on a 429 (server rate limit, Cluster 172).
        self.retry_after: Optional[float] = None

    @property
    def is_conflict(self) -> bool:  # 409
        return self.status == 409

    @property
    def is_cursor_too_old(self) -> bool:
        """409 + must_refetch / cursor-too-old — fail loud, never clamp."""
        if self.status != 409:
            return False
        body = self.body if isinstance(self.body, dict) else {}
        if body.get("must_refetch") is True:
            return True
        return body.get("type") in (
            "https://maidan.dev/problems/cursor-too-old",
            "cursor_too_old",
        )

    @property
    def is_forbidden(self) -> bool:  # 403 (missing capability / channel access)
        return self.status == 403

    @property
    def is_rate_limited(self) -> bool:  # 429
        return self.status == 429


class Subscription:
    """A live subscription handle. Call :meth:`close` to stop it."""

    def __init__(self, conn: "_WebSocketConn"):
        self._conn = conn

    def close(self) -> None:
        self._conn.close()


class _Workspaces:
    def __init__(self, c: "Client"):
        self._c = c

    def create(self, name: str) -> Any:
        return self._c._req("POST", "/workspaces", {"name": name})

    def get(self, workspace_id: str) -> Any:
        return self._c._req("GET", f"/workspaces/{workspace_id}")

    def import_(self, bundle: Any, mode: Optional[str] = None) -> Any:
        """Admin-only (``token:admin``). ``import`` is reserved, hence the underscore."""
        path = "/workspaces/import" + (f"?mode={mode}" if mode else "")
        return self._c._req("POST", path, bundle)


class _Channels:
    def __init__(self, c: "Client"):
        self._c = c

    def list(self, workspace_id: str) -> Any:
        return self._c._req("GET", f"/workspaces/{workspace_id}/channels")

    def create(self, workspace_id: str, name: str, private: bool = False) -> Any:
        return self._c._req(
            "POST", f"/workspaces/{workspace_id}/channels", {"name": name, "private": private}
        )


class _Members:
    """Member provisioning.

    ``create`` is the unauthenticated seed route, present only on a server built
    with the ``bootstrap`` feature — how a fresh workspace gets its first agents.
    A production deployment turns it off and provisions through ``maidan init``
    plus :class:`_Tokens`.
    """

    def __init__(self, c: "Client"):
        self._c = c

    def create(self, workspace_id: str, handle: str, kind: str = "agent", display_name: Optional[str] = None) -> Any:
        body: dict = {"handle": handle, "kind": kind}
        if display_name is not None:
            body["display_name"] = display_name
        return self._c._req("POST", f"/workspaces/{workspace_id}/members", body)

    def list(self, workspace_id: str) -> Any:
        return self._c._req("GET", f"/workspaces/{workspace_id}/members")


class _Tokens:
    """Per-agent bearer tokens. Requires ``token:admin`` — the capability the
    admin token from ``maidan init`` carries.

    ``mint`` returns the secret **once**, in the response; it is never
    retrievable again. ``list`` returns metadata only.
    """

    def __init__(self, c: "Client"):
        self._c = c

    def mint(
        self,
        workspace_id: str,
        member_id: str,
        capabilities: Optional[list] = None,
        *,
        label: Optional[str] = None,
        capability_set: Optional[str] = None,
        expires_at: Optional[str] = None,
    ) -> Any:
        """POST /workspaces/{wid}/members/{mid}/tokens.

        Pass ``capabilities`` to grant exactly those, or ``capability_set``
        (``maidan.agent.worker`` / ``maidan.human.admin``) for a named set —
        together they are a progressive grant, so the requested capabilities
        must be a subset of the set.
        """
        body: dict = {"capabilities": capabilities or []}
        if label is not None:
            body["label"] = label
        if capability_set is not None:
            body["capability_set"] = capability_set
        if expires_at is not None:
            body["expires_at"] = expires_at
        return self._c._req("POST", f"/workspaces/{workspace_id}/members/{member_id}/tokens", body)

    def list(self, workspace_id: str, member_id: str) -> Any:
        return self._c._req("GET", f"/workspaces/{workspace_id}/members/{member_id}/tokens")


class _Threads:
    def __init__(self, c: "Client"):
        self._c = c

    def create(self, channel_id: str, title: str) -> Any:
        return self._c._req("POST", f"/channels/{channel_id}/threads", {"title": title})

    def get(self, thread_id: str) -> Any:
        return self._c._req("GET", f"/threads/{thread_id}")

    def context(self, thread_id: str, query: Optional[dict] = None) -> Any:
        return self._c._req("GET", f"/threads/{thread_id}/context{_qs(query)}")

    def transition(self, thread_id: str, body: Any) -> Any:
        return self._c._req("POST", f"/threads/{thread_id}", body)

    def set_result(self, thread_id: str, result: Any) -> Any:
        return self._c._req("PUT", f"/threads/{thread_id}/result", {"result": result})

    def get_result(self, thread_id: str) -> Any:
        return self._c._req("GET", f"/threads/{thread_id}/result")


class _Messages:
    def __init__(self, c: "Client"):
        self._c = c

    def list(self, thread_id: str, query: Optional[dict] = None) -> Any:
        return self._c._req("GET", f"/threads/{thread_id}/messages{_qs(query)}")

    def post(self, thread_id: str, author_id: str, body: str) -> Any:
        return self._c._req(
            "POST", f"/threads/{thread_id}/messages", {"author_id": author_id, "body": body}
        )


class _Artifacts:
    def __init__(self, c: "Client"):
        self._c = c

    def upload(self, data: bytes, kind: str) -> Any:
        return self._c._req_raw("POST", f"/artifacts?kind={urllib.parse.quote(kind)}", data)

    def get(self, sha: str) -> bytes:
        return self._c._req_raw("GET", f"/artifacts/{sha}")

    def meta(self, sha: str) -> Any:
        return self._c._req("GET", f"/artifacts/{sha}/meta")


class Client:
    """A Maidan v1 client over REST + WebSocket.

    ``base_url`` / ``token`` default to ``MAIDAN_URL`` / ``MAIDAN_TOKEN``.
    ``client.mcp_url`` is ``{base_url}/mcp/streamable`` (a string — no MCP dependency).
    """

    def __init__(self, base_url: Optional[str] = None, token: Optional[str] = None, *, timeout: float = 30.0):
        self.base_url = (base_url or os.environ.get("MAIDAN_URL") or "http://127.0.0.1:8080").rstrip("/")
        self.token = token or os.environ.get("MAIDAN_TOKEN") or ""
        self.timeout = timeout
        # MCP is a URL, not a dependency (docs/Client Contract.md §0).
        self.mcp_url = f"{self.base_url}/mcp/streamable"
        # Last seen Maidan-Room-LSN (event-log high-water). Not a WAL token.
        self.last_room_lsn: Optional[int] = None

        self.workspaces = _Workspaces(self)
        self.members = _Members(self)
        self.tokens = _Tokens(self)
        self.channels = _Channels(self)
        self.threads = _Threads(self)
        self.messages = _Messages(self)
        self.artifacts = _Artifacts(self)

    # --- hero methods ---
    def claim_next_thread(self, channel_id: str, body: Optional[dict] = None) -> Any:
        """POST /channels/{cid}/threads/claim-next — readiness/skill/lease-aware.

        Returns the claimed thread with its fields at the TOP level (plus a
        content-addressed ``pin``), or ``None`` when nothing is claimable. There
        is no nested ``thread`` key: ``claim["id"]``, not ``claim["thread"]["id"]``.
        """
        return self._req("POST", f"/channels/{channel_id}/threads/claim-next", body or {})

    def renew_claim(
        self,
        thread_id: str,
        member_id: str,
        claim_lease_id: str,
        lease_secs: int = 300,
    ) -> Any:
        """POST /threads/{id}/claim/renew — holder-only lease heartbeat.

        ``claim_lease_id`` is the fencing token the claim handed back
        (``claim["claim_lease_id"]``). Presenting it is what stops a stale holder
        from extending a lease the next owner has already taken over.
        """
        return self._req(
            "POST",
            f"/threads/{thread_id}/claim/renew",
            {
                "member_id": member_id,
                "claim_lease_id": claim_lease_id,
                "lease_secs": lease_secs,
            },
        )

    def _capture_room_lsn(self, headers: Any) -> None:
        raw = None
        if headers is not None:
            raw = headers.get("Maidan-Room-LSN") or headers.get("maidan-room-lsn")
        parsed = parse_room_lsn(raw)
        if parsed is not None:
            self.last_room_lsn = parsed

    # --- HTTP core ---
    def _req(self, method: str, path: str, body: Any = None) -> Any:
        headers = {"authorization": f"Bearer {self.token}"}
        data = None
        if body is not None:
            headers["content-type"] = "application/json"
            data = json.dumps(body).encode("utf-8")
        req = urllib.request.Request(f"{self.base_url}{path}", data=data, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                self._capture_room_lsn(resp.headers)
                raw = resp.read()
                if resp.status == 204 or not raw:
                    return None
                return json.loads(raw.decode("utf-8"))
        except urllib.error.HTTPError as e:
            self._raise(e)

    def _req_raw(self, method: str, path: str, body: Optional[bytes] = None) -> Any:
        headers = {"authorization": f"Bearer {self.token}"}
        req = urllib.request.Request(f"{self.base_url}{path}", data=body, method=method, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=self.timeout) as resp:
                self._capture_room_lsn(resp.headers)
                raw = resp.read()
                if method == "GET":
                    return raw
                if resp.status == 204 or not raw:
                    return None
                return json.loads(raw.decode("utf-8"))
        except urllib.error.HTTPError as e:
            self._raise(e)

    def _raise(self, e: urllib.error.HTTPError) -> None:
        self._capture_room_lsn(e.headers)
        text = ""
        try:
            text = e.read().decode("utf-8")
        except Exception:
            pass
        try:
            parsed = json.loads(text) if text else None
        except ValueError:
            parsed = text
        err = MaidanError(e.code, parsed)
        if e.code == 429:
            ra = e.headers.get("retry-after")
            if ra:
                try:
                    err.retry_after = float(ra)
                except ValueError:
                    pass
        raise err

    # --- WebSocket subscribe ---
    def list_events(self, workspace_id: str, query: Optional[dict] = None) -> Any:
        """GET /workspaces/{id}/events — projector-shaped HTTP backfill."""
        return self._req("GET", f"/workspaces/{workspace_id}/events{_qs(query)}")

    def subscribe(
        self,
        filter: Optional[dict],
        on_event: Callable[[dict], None],
        on_error: Optional[Callable[[Exception], None]] = None,
        *,
        after_id: int = 0,
        consumer_id: Optional[str] = None,
    ) -> Subscription:
        """Subscribe to the event stream over WebSocket. ``filter`` follows
        contracts/ws-subscribe-filter.schema.json (set ``workspace_id`` for replay).
        Benign control frames (subscribe_ack, schema_version, replay_*) are skipped.
        ``type: cursor_too_old`` is delivered (then the socket closes) — never a
        silent skip. Unknown kinds are still delivered.
        """
        ws_url = self.base_url.replace("http", "ws", 1) + "/ws/subscribe"

        def _dispatch(text: str) -> None:
            try:
                frame = json.loads(text)
            except ValueError:
                return
            if not isinstance(frame, dict):
                return
            if frame.get("type") == "cursor_too_old":
                on_event(frame)
                return
            if frame.get("type") is not None:
                return
            if isinstance(frame.get("kind"), str):
                on_event(frame)

        conn = _WebSocketConn(ws_url, _dispatch, on_error)
        conn.connect()
        payload: dict = {"filter": filter or {}, "token": self.token}
        if after_id:
            payload["after_id"] = after_id
        if consumer_id:
            payload["consumer_id"] = consumer_id
        conn.send_text(json.dumps(payload))
        conn.start()
        return Subscription(conn)

    def follow(
        self,
        workspace_id: str,
        on_event: Callable[[dict], None],
        *,
        after_id: int = 0,
        channel_id: Optional[str] = None,
        thread_id: Optional[str] = None,
        types: Optional[list] = None,
        consumer_id: Optional[str] = None,
        page_limit: int = 100,
        on_error: Optional[Callable[[Exception], None]] = None,
    ) -> Subscription:
        """HTTP backfill then WS cutover. A 409 must_refetch raises
        :attr:`MaidanError.is_cursor_too_old` — never clamped.
        """
        after = after_id
        limit = page_limit if page_limit > 0 else 100
        while True:
            q: dict = {"after_id": after, "limit": limit}
            if channel_id:
                q["channel_id"] = channel_id
            if thread_id:
                q["thread_id"] = thread_id
            if types:
                q["types"] = ",".join(types)
            if consumer_id:
                q["consumer_id"] = consumer_id
            page = self.list_events(workspace_id, q) or []
            if not page:
                break
            for row in page:
                rid = row.get("id") if isinstance(row, dict) else None
                if isinstance(rid, int):
                    after = max(after, rid)
                on_event(_normalize_stored(row))
            if len(page) < limit:
                break
        filt: dict = {"workspace_id": workspace_id}
        if channel_id:
            filt["channel_id"] = channel_id
        if thread_id:
            filt["thread_id"] = thread_id
        if types:
            filt["kinds"] = types
        return self.subscribe(
            filt,
            on_event,
            on_error,
            after_id=after,
            consumer_id=consumer_id,
        )

    def _wait_for_kind(self, filter: dict, kind: str, timeout: float = 30.0) -> Optional[dict]:
        got: dict = {}
        done = threading.Event()

        def _on(event: dict) -> None:
            got["event"] = event
            done.set()

        sub = self.subscribe({**filter, "kinds": [kind]}, _on)
        try:
            done.wait(timeout)
            return got.get("event")
        finally:
            sub.close()

    def wait_for_result(self, thread_id: str, workspace_id: str, timeout: float = 30.0) -> Optional[dict]:
        return self._wait_for_kind({"workspace_id": workspace_id, "thread_id": thread_id}, "thread_result_set", timeout)

    def wait_for_mention(self, member_id: str, workspace_id: str, timeout: float = 30.0) -> Optional[dict]:
        return self._wait_for_kind({"workspace_id": workspace_id, "member_id": member_id}, "mention_recorded", timeout)

    def wait_for_ready(self, workspace_id: str, channel_id: Optional[str] = None, timeout: float = 30.0) -> Optional[dict]:
        f = {"workspace_id": workspace_id}
        if channel_id:
            f["channel_id"] = channel_id
        return self._wait_for_kind(f, "thread_ready", timeout)


def _normalize_stored(row: Any) -> dict:
    """HTTP StoredEvent (`id` + nested payload) → live bus shape (`log_id` + flat)."""
    if not isinstance(row, dict):
        return {"raw": row}
    payload = row.get("payload")
    out = dict(payload) if isinstance(payload, dict) else dict(row)
    rid = row.get("id", row.get("log_id"))
    if rid is not None:
        out["log_id"] = rid
    for key in ("kind", "workspace_id", "channel_id", "thread_id"):
        if key not in out and key in row:
            out[key] = row[key]
    return out


def _qs(query: Optional[dict]) -> str:
    if not query:
        return ""
    s = urllib.parse.urlencode(query)
    return f"?{s}" if s else ""


class _WebSocketConn:
    """A minimal RFC-6455 client: handshake, one masked send, a receive loop for
    text frames. Enough for Maidan's subscribe (small JSON text frames); handles
    ping (auto-pong), close, and fragmentation. No third-party dependency."""

    def __init__(self, url: str, on_text: Callable[[str], None], on_error: Optional[Callable[[Exception], None]]):
        self._url = url
        self._on_text = on_text
        self._on_error = on_error
        self._sock: Optional[socket.socket] = None
        self._buf = b""
        self._closed = False
        self._thread: Optional[threading.Thread] = None
        self._send_lock = threading.Lock()

    def connect(self) -> None:
        parsed = urllib.parse.urlparse(self._url)
        secure = parsed.scheme == "wss"
        host = parsed.hostname or "127.0.0.1"
        port = parsed.port or (443 if secure else 80)
        path = parsed.path or "/"
        if parsed.query:
            path += "?" + parsed.query

        raw = socket.create_connection((host, port), timeout=30)
        if secure:
            ctx = ssl.create_default_context()
            raw = ctx.wrap_socket(raw, server_hostname=host)
        self._sock = raw

        key = base64.b64encode(os.urandom(16)).decode("ascii")
        req = (
            f"GET {path} HTTP/1.1\r\n"
            f"Host: {host}:{port}\r\n"
            "Upgrade: websocket\r\n"
            "Connection: Upgrade\r\n"
            f"Sec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        )
        raw.sendall(req.encode("ascii"))

        # Read the handshake response headers; keep any trailing frame bytes.
        while b"\r\n\r\n" not in self._buf:
            chunk = raw.recv(4096)
            if not chunk:
                raise MaidanError(0, None, "WebSocket handshake closed early")
            self._buf += chunk
        head, self._buf = self._buf.split(b"\r\n\r\n", 1)
        status_line = head.split(b"\r\n", 1)[0].decode("latin-1", "replace")
        if "101" not in status_line:
            raise MaidanError(0, status_line, f"WebSocket handshake failed: {status_line}")

    def start(self) -> None:
        self._thread = threading.Thread(target=self._read_loop, daemon=True)
        self._thread.start()

    def _recv_exactly(self, n: int) -> bytes:
        while len(self._buf) < n:
            chunk = self._sock.recv(4096)  # type: ignore[union-attr]
            if not chunk:
                raise ConnectionError("WebSocket closed")
            self._buf += chunk
        out, self._buf = self._buf[:n], self._buf[n:]
        return out

    def _read_loop(self) -> None:
        message = bytearray()
        message_opcode = None
        try:
            while not self._closed:
                b0, b1 = self._recv_exactly(2)
                fin = b0 & 0x80
                opcode = b0 & 0x0F
                masked = b1 & 0x80
                length = b1 & 0x7F
                if length == 126:
                    (length,) = struct.unpack(">H", self._recv_exactly(2))
                elif length == 127:
                    (length,) = struct.unpack(">Q", self._recv_exactly(8))
                mask = self._recv_exactly(4) if masked else b""
                payload = self._recv_exactly(length) if length else b""
                if masked:
                    payload = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))

                if opcode == 0x8:  # close
                    break
                if opcode == 0x9:  # ping -> pong
                    self._send_frame(0xA, payload)
                    continue
                if opcode == 0xA:  # pong
                    continue
                if opcode in (0x1, 0x2):  # text / binary
                    message = bytearray(payload)
                    message_opcode = opcode
                elif opcode == 0x0:  # continuation
                    message.extend(payload)
                if fin and message_opcode == 0x1:
                    self._on_text(message.decode("utf-8", "replace"))
                    message = bytearray()
                    message_opcode = None
                elif fin:
                    message = bytearray()
                    message_opcode = None
        except Exception as exc:  # noqa: BLE001 — surface, don't crash the thread
            if not self._closed and self._on_error:
                self._on_error(exc)
        finally:
            self._shutdown_socket()

    def send_text(self, text: str) -> None:
        self._send_frame(0x1, text.encode("utf-8"))

    def _send_frame(self, opcode: int, payload: bytes) -> None:
        if self._sock is None:
            return
        header = bytearray()
        header.append(0x80 | opcode)  # FIN + opcode
        length = len(payload)
        mask_bit = 0x80  # client frames MUST be masked (RFC 6455)
        if length < 126:
            header.append(mask_bit | length)
        elif length < (1 << 16):
            header.append(mask_bit | 126)
            header.extend(struct.pack(">H", length))
        else:
            header.append(mask_bit | 127)
            header.extend(struct.pack(">Q", length))
        mask = os.urandom(4)
        header.extend(mask)
        masked = bytes(b ^ mask[i % 4] for i, b in enumerate(payload))
        with self._send_lock:
            try:
                self._sock.sendall(bytes(header) + masked)
            except OSError:
                pass

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            self._send_frame(0x8, b"")  # close frame
        except Exception:
            pass
        self._shutdown_socket()

    def _shutdown_socket(self) -> None:
        if self._sock is not None:
            try:
                self._sock.close()
            except OSError:
                pass
            self._sock = None
