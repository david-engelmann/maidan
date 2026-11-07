"""Offline WebSocket and dripping-HTTP tests. No Maidan server."""

import base64
import hashlib
import json
import socket
import struct
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import pytest

from maidan import Client, MaidanError, __version__
from maidan.client import _WS_GUID, _websocket_url

GUID = _WS_GUID


def _accept(key: str) -> str:
    return base64.b64encode(hashlib.sha1((key + GUID).encode("ascii")).digest()).decode("ascii")


def _frame(text: str) -> bytes:
    payload = text.encode("utf-8")
    n = len(payload)
    if n < 126:
        header = bytes([0x81, n])
    else:
        header = bytes([0x81, 126]) + struct.pack(">H", n)
    return header + payload


def _read_headers(conn: socket.socket) -> bytes:
    buf = b""
    while b"\r\n\r\n" not in buf and len(buf) < 65536:
        chunk = conn.recv(4096)
        if not chunk:
            break
        buf += chunk
    return buf


def _serve_once(handler) -> tuple[str, threading.Thread]:
    ready = threading.Event()
    holder: dict = {}

    def run() -> None:
        srv = socket.socket()
        srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        srv.bind(("127.0.0.1", 0))
        srv.listen(1)
        holder["srv"] = srv
        holder["port"] = srv.getsockname()[1]
        ready.set()
        srv.settimeout(5)
        try:
            conn, _ = srv.accept()
        except OSError:
            return
        holder["conn"] = conn
        try:
            handler(conn)
        finally:
            try:
                conn.close()
            except OSError:
                pass
            srv.close()

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    ready.wait(2)
    return f"http://127.0.0.1:{holder['port']}", thread


def _upgrade(conn: socket.socket, status: str, extra: str = "") -> bytes:
    raw = _read_headers(conn)
    key = ""
    for line in raw.decode("latin-1", "replace").split("\r\n"):
        if line.lower().startswith("sec-websocket-key:"):
            key = line.split(":", 1)[1].strip()
    accept = _accept(key) if key else "nope"
    body = (
        f"HTTP/1.1 {status}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Accept: {accept}\r\n"
        f"{extra}\r\n"
    )
    conn.sendall(body.encode("ascii"))
    return raw


def test_the_scheme_change_does_not_eat_http_inside_the_host():
    assert _websocket_url("http://http-gateway.local:8080") == "ws://http-gateway.local:8080/ws/subscribe"
    assert _websocket_url("https://maidan.world") == "wss://maidan.world/ws/subscribe"
    with pytest.raises(MaidanError):
        _websocket_url("ftp://files.example")


def test_an_idle_subscription_stays_up_and_close_stops_the_reader():
    got = threading.Event()
    seen: dict = {}

    def handler(conn: socket.socket) -> None:
        _upgrade(conn, "101 Switching Protocols")
        time.sleep(0.7)
        conn.sendall(_frame(json.dumps({"kind": "message_posted", "log_id": 1})))
        try:
            while conn.recv(64):
                pass
        except OSError:
            pass

    base, thread = _serve_once(handler)
    errors: list = []
    client = Client(base, "t", timeout=0.25)
    sub = client.subscribe({"workspace_id": "w"}, lambda event: seen.update(event) or got.set(), errors.append)
    assert got.wait(2), errors
    assert seen["kind"] == "message_posted"
    sub.close()
    reader = sub._conn._thread
    assert reader is not None
    reader.join(1)
    assert not reader.is_alive()
    thread.join(2)


def test_the_handshake_names_itself_and_refuses_a_false_101():
    def handler(conn: socket.socket) -> None:
        raw = _upgrade(conn, "403 Forbidden (error 1010)")
        text = raw.decode("latin-1", "replace")
        assert f"User-Agent: maidan-python/{__version__}" in text
        try:
            while conn.recv(64):
                pass
        except OSError:
            pass

    base, thread = _serve_once(handler)
    with pytest.raises(MaidanError) as ei:
        Client(base, "t", timeout=2).subscribe({}, lambda _e: None)
    assert "403" in str(ei.value)
    assert "1010" in str(ei.value)
    thread.join(2)


def test_a_bad_accept_and_a_huge_header_and_a_slow_handshake_fail_closed():
    def bad_accept(conn: socket.socket) -> None:
        _read_headers(conn)
        conn.sendall(b"HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nSec-WebSocket-Accept: nope\r\n\r\n")
        try:
            while conn.recv(64):
                pass
        except OSError:
            pass

    base, thread = _serve_once(bad_accept)
    with pytest.raises(MaidanError):
        Client(base, "t", timeout=2).subscribe({}, lambda _e: None)
    thread.join(2)

    def huge(conn: socket.socket) -> None:
        _read_headers(conn)
        conn.sendall(b"HTTP/1.1 101 Switching Protocols\r\n" + (b"X" * 70000))
        try:
            while conn.recv(64):
                pass
        except OSError:
            pass

    base, thread = _serve_once(huge)
    with pytest.raises(MaidanError):
        Client(base, "t", timeout=2).subscribe({}, lambda _e: None)
    thread.join(2)

    closed = threading.Event()

    def slow(conn: socket.socket) -> None:
        conn.sendall(b"H")
        time.sleep(0.5)
        try:
            conn.recv(16)
        except OSError:
            closed.set()

    base, thread = _serve_once(slow)
    started = time.monotonic()
    with pytest.raises(MaidanError):
        Client(base, "t", timeout=0.2).subscribe({}, lambda _e: None)
    assert time.monotonic() - started < 1.0
    thread.join(2)


def test_a_dripping_http_body_stops_and_a_retry_reuses_the_key_and_bytes():
    seen: list[tuple[str, bytes]] = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            n = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(n)
            seen.append((self.headers.get("Idempotency-Key") or "", body))
            payload = b'{"ok":true}'
            if len(seen) == 1:
                self.send_response(200)
                self.send_header("Content-Length", "400")
                self.send_header("Connection", "close")
                self.end_headers()
                try:
                    for _ in range(30):
                        self.wfile.write(b"xxxx")
                        self.wfile.flush()
                        time.sleep(0.05)
                except (BrokenPipeError, ConnectionResetError, OSError):
                    return
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.send_header("Connection", "close")
            self.end_headers()
            self.wfile.write(payload[:4])
            self.wfile.flush()
            time.sleep(0.02)
            self.wfile.write(payload[4:])

        def log_message(self, *_):
            return

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    url = f"http://127.0.0.1:{server.server_address[1]}"
    try:
        slow = Client(url, "t", timeout=0.15, max_retries=0)
        with pytest.raises(MaidanError) as ei:
            slow._send("POST", "/write", {}, b'{"n":1}')
        assert ei.value.status == 0
        assert "TimeoutError" in str(ei.value) or "timed out" in str(ei.value).lower()

        seen.clear()
        again = Client(url, "t", timeout=0.15, max_retries=1)
        again._sleep = lambda _s: None
        status, _headers, raw = again._send("POST", "/write", {}, b'{"n":1}')
        assert status == 200
        assert json.loads(raw) == {"ok": True}
        assert len(seen) == 2
        assert seen[0][0] and seen[0][0] == seen[1][0]
        assert seen[0][1] == seen[1][1] == b'{"n":1}'
    finally:
        server.shutdown()
