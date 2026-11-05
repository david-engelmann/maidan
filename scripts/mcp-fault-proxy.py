#!/usr/bin/env python3
"""A fault-injecting HTTP proxy for testing Maidan clients.

Put it in front of a Maidan (`/mcp/streamable` and the REST routes alike) and
point a claimant at it. Each request is matched against a plan of rules; the
first rule that matches, and still has faults to give, decides what the client
gets instead of a clean answer:

    latency      wait `secs`, then forward (`after: true` waits after the answer)
    timeout      hold the connection `secs` without answering, then close it;
                 the upstream never sees the request
    drop         close the connection without an answer; upstream never sees it
    drop_after   forward, so the write lands, then close without an answer
    status       answer `status` (500, 502, 503, ...) without forwarding, with
                 `retry_after` as the Retry-After header when it is set
    truncate     forward, then send the first `bytes` of the answer. With
                 `keep_length` the Content-Length is the full answer's, so the
                 client sees the connection end early; without it the
                 Content-Length matches, so the client reads broken JSON
    rpc_error    answer a JSON-RPC error (`code`, `message`) to the request's
                 id without forwarding: a fault inside a 200
    drift        forward, then change the answer's schema: `drop` lists
                 fields to remove, `add` fields to add, `set` fields to give
                 another value (a wrong type). An MCP tool result is changed
                 inside its text content and `structuredContent`

A rule matches on any of `n` (the request's number through the proxy, from
1), `method` (HTTP), `path` (a regular expression searched in the path),
`rpc` (the JSON-RPC method) and `tool` (the tool a `tools/call` names).
`skip` lets that many matching requests through first; `times` is how many it
faults after that (default 1, -1 for every one).

    python3 scripts/mcp-fault-proxy.py --upstream http://127.0.0.1:8080 \\
        --listen 127.0.0.1:9090 --plan plan.json

`plan.json` is a list of rules:

    [{"tool": "claim_next_workspace_thread", "fault": {"kind": "drop_after"}},
     {"path": "/claim/renew$", "times": -1, "fault": {"kind": "status", "status": 503}}]

One JSON line per request on stderr. Tests import it and use `FaultProxy`.
Python 3.11+, standard library only.
"""

from __future__ import annotations

import argparse
import copy
import http.client
import json
import re
import sys
import threading
import time
import urllib.parse
from dataclasses import dataclass, field
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Any

# Headers that belong to one connection, not to the request it carries.
HOP_BY_HOP = frozenset({
    "connection", "keep-alive", "proxy-authenticate", "proxy-authorization",
    "te", "trailers", "transfer-encoding", "upgrade", "host", "content-length",
})
KINDS = frozenset({"latency", "timeout", "drop", "drop_after", "status", "truncate", "rpc_error", "drift"})


@dataclass
class Rule:
    fault: dict[str, Any]
    n: int | None = None
    method: str | None = None
    path: str | None = None
    rpc: str | None = None
    tool: str | None = None
    skip: int = 0
    times: int = 1
    seen: int = 0

    @classmethod
    def from_dict(cls, raw: dict[str, Any]) -> "Rule":
        fault = raw.get("fault")
        if not isinstance(fault, dict) or fault.get("kind") not in KINDS:
            raise ValueError(f"a rule needs a fault whose kind is one of {sorted(KINDS)}: {raw}")
        known = {"n", "method", "path", "rpc", "tool", "skip", "times"}
        return cls(fault=fault, **{k: v for k, v in raw.items() if k in known})

    def matches(self, req: "Seen") -> bool:
        return ((self.n is None or self.n == req.n)
                and (self.method is None or self.method == req.method)
                and (self.path is None or re.search(self.path, req.path) is not None)
                and (self.rpc is None or self.rpc == req.rpc)
                and (self.tool is None or self.tool == req.tool))

    def take(self) -> bool:
        """Whether this matching request gets the fault; counts it either way."""
        self.seen += 1
        if self.seen <= self.skip:
            return False
        return self.times < 0 or self.seen - self.skip <= self.times


@dataclass
class Seen:
    """One request through the proxy, as the log records it."""
    n: int
    method: str
    path: str
    rpc: str | None
    tool: str | None
    headers: dict[str, str]
    body: Any
    fault: str | None = None
    forwarded: bool = False
    status: int | None = None
    at: float = field(default_factory=time.monotonic)

    def line(self) -> dict[str, Any]:
        return {"n": self.n, "method": self.method, "path": self.path, "rpc": self.rpc, "tool": self.tool,
                "fault": self.fault, "forwarded": self.forwarded, "status": self.status}


def rpc_of(body: Any) -> tuple[str | None, str | None]:
    if not isinstance(body, dict) or body.get("jsonrpc") != "2.0":
        return None, None
    method = body.get("method") if isinstance(body.get("method"), str) else None
    params = body.get("params")
    tool = params.get("name") if method == "tools/call" and isinstance(params, dict) else None
    return method, tool if isinstance(tool, str) else None


def drift(value: Any, spec: dict[str, Any]) -> Any:
    """`value` with `spec`'s fields dropped, added and set, when it is an object."""
    if not isinstance(value, dict):
        return value
    out = copy.deepcopy(value)
    for name in spec.get("drop", []):
        out.pop(name, None)
    out.update(spec.get("add", {}))
    out.update(spec.get("set", {}))
    return out


def drift_answer(raw: bytes, spec: dict[str, Any]) -> bytes:
    """The answer with its schema drifted. A JSON-RPC tool result drifts in
    the tool's own value (its text content and `structuredContent`); any other
    JSON object drifts at the top."""
    try:
        doc = json.loads(raw)
    except ValueError:
        return raw
    result = doc.get("result") if isinstance(doc, dict) and doc.get("jsonrpc") == "2.0" else None
    if isinstance(result, dict) and isinstance(result.get("content"), list):
        for block in result["content"]:
            if isinstance(block, dict) and block.get("type") == "text":
                try:
                    block["text"] = json.dumps(drift(json.loads(block["text"]), spec))
                except (ValueError, TypeError):
                    pass
        if "structuredContent" in result:
            result["structuredContent"] = drift(result["structuredContent"], spec)
    else:
        doc = drift(doc, spec)
    return json.dumps(doc).encode()


class FaultProxy:
    """A threaded proxy on 127.0.0.1 in front of `upstream`; `url` is its
    address. `plan` is a list of rules (`Rule` or their dicts), replaceable
    with `set_plan`. `seen` records every request."""

    def __init__(self, upstream: str, plan: list[Any] | None = None, host: str = "127.0.0.1",
                 port: int = 0, log=None) -> None:
        parts = urllib.parse.urlsplit(upstream)
        if parts.scheme not in ("http", "https"):
            raise ValueError(f"upstream must be http or https: {upstream}")
        self.upstream = parts
        self.lock = threading.Lock()
        self.seen: list[Seen] = []
        self.rules: list[Rule] = []
        self.log = log
        self.set_plan(plan or [])
        proxy = self

        class Handler(BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def log_message(self, *_: Any) -> None:
                pass

            def handle_one(self) -> None:
                proxy.handle(self)

            do_GET = do_POST = do_PUT = do_PATCH = do_DELETE = handle_one

        self.server = ThreadingHTTPServer((host, port), Handler)
        self.server.daemon_threads = True
        self.url = f"http://{host}:{self.server.server_address[1]}"
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()

    def set_plan(self, plan: list[Any]) -> None:
        with self.lock:
            self.rules = [r if isinstance(r, Rule) else Rule.from_dict(r) for r in plan]

    def close(self) -> None:
        self.server.shutdown()
        self.server.server_close()

    def requests(self, path: str | None = None, *, forwarded: bool | None = None) -> list[Seen]:
        with self.lock:
            return [s for s in self.seen
                    if (path is None or re.search(path, s.path))
                    and (forwarded is None or s.forwarded == forwarded)]

    def _fault_for(self, seen: Seen) -> dict[str, Any] | None:
        with self.lock:
            self.seen.append(seen)
            for rule in self.rules:
                if rule.matches(seen) and rule.take():
                    return rule.fault
        return None

    def _forward(self, h: BaseHTTPRequestHandler, raw_body: bytes) -> tuple[int, list[tuple[str, str]], bytes]:
        cls = http.client.HTTPSConnection if self.upstream.scheme == "https" else http.client.HTTPConnection
        conn = cls(self.upstream.hostname, self.upstream.port, timeout=120)
        headers = {k: v for k, v in h.headers.items() if k.lower() not in HOP_BY_HOP}
        base = self.upstream.path.rstrip("/")
        try:
            conn.request(h.command, base + h.path, body=raw_body if raw_body or h.command != "GET" else None,
                         headers=headers)
            res = conn.getresponse()
            return res.status, [(k, v) for k, v in res.getheaders() if k.lower() not in HOP_BY_HOP], res.read()
        finally:
            conn.close()

    @staticmethod
    def _answer(h: BaseHTTPRequestHandler, status: int, headers: list[tuple[str, str]], body: bytes,
                length: int | None = None) -> None:
        h.send_response(status)
        for k, v in headers:
            h.send_header(k, v)
        h.send_header("Content-Length", str(len(body) if length is None else length))
        h.send_header("Connection", "close")
        h.end_headers()
        h.wfile.write(body)
        h.wfile.flush()
        h.close_connection = True

    @staticmethod
    def _hang_up(h: BaseHTTPRequestHandler) -> None:
        h.close_connection = True
        try:
            h.wfile.flush()
            h.connection.shutdown(2)
        except OSError:
            pass

    def handle(self, h: BaseHTTPRequestHandler) -> None:
        n = int(h.headers.get("Content-Length") or 0)
        raw_body = h.rfile.read(n) if n else b""
        try:
            body = json.loads(raw_body) if raw_body else None
        except ValueError:
            body = None
        rpc, tool = rpc_of(body)
        with self.lock:
            number = len(self.seen) + 1
        seen = Seen(number, h.command, urllib.parse.urlsplit(h.path).path, rpc, tool,
                    {k.lower(): v for k, v in h.headers.items()}, body)
        fault = self._fault_for(seen)
        kind = fault["kind"] if fault else None
        seen.fault = kind
        try:
            if kind == "latency" and not fault.get("after"):
                time.sleep(float(fault.get("secs", 1)))
            if kind in ("timeout", "drop"):
                if kind == "timeout":
                    time.sleep(float(fault.get("secs", 5)))
                self._hang_up(h)
                return
            if kind == "status":
                status = int(fault.get("status", 503))
                headers = [("Content-Type", "application/problem+json")]
                if fault.get("retry_after") is not None:
                    headers.append(("Retry-After", str(fault["retry_after"])))
                problem = {"type": "about:blank", "title": "injected fault", "status": status,
                           "detail": f"fault proxy: {status}"}
                seen.status = status
                self._answer(h, status, headers, json.dumps(problem).encode())
                return
            if kind == "rpc_error":
                rid = body.get("id") if isinstance(body, dict) else None
                doc = {"jsonrpc": "2.0", "id": rid, "error": {
                    "code": int(fault.get("code", -32603)), "message": fault.get("message", "fault proxy: internal error")}}
                seen.status = 200
                self._answer(h, 200, [("Content-Type", "application/json")], json.dumps(doc).encode())
                return
            status, headers, answer = self._forward(h, raw_body)
            seen.forwarded, seen.status = True, status
            if kind == "latency":
                time.sleep(float(fault.get("secs", 1)))
            if kind == "drop_after":
                self._hang_up(h)
                return
            if kind == "truncate":
                cut = answer[: int(fault.get("bytes", max(1, len(answer) // 2)))]
                self._answer(h, status, headers, cut, len(answer) if fault.get("keep_length") else None)
                return
            if kind == "drift":
                answer = drift_answer(answer, fault)
            self._answer(h, status, headers, answer)
        except (BrokenPipeError, ConnectionResetError):
            # The client gave up first (its own timeout); nothing to answer.
            h.close_connection = True
        finally:
            if self.log is not None:
                print(json.dumps(seen.line()), file=self.log, flush=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--upstream", required=True, help="the Maidan to front, e.g. http://127.0.0.1:8080")
    parser.add_argument("--listen", default="127.0.0.1:0", help="host:port (port 0 picks a free one)")
    parser.add_argument("--plan", help="a JSON file holding the list of rules (default: no faults)")
    args = parser.parse_args(argv)
    host, _, port = args.listen.rpartition(":")
    plan = json.loads(open(args.plan).read()) if args.plan else []
    proxy = FaultProxy(args.upstream, plan, host=host or "127.0.0.1", port=int(port or 0), log=sys.stderr)
    print(f"fault proxy on {proxy.url} -> {args.upstream}", file=sys.stderr, flush=True)
    try:
        proxy.thread.join()
    except KeyboardInterrupt:
        proxy.close()
    return 0


if __name__ == "__main__":
    sys.exit(main())
