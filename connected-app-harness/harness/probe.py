"""Server-side probe: the strongest assertions are server-side.

Tails the Maidan server log and waits for evidence that an MCP tool was
called. Correlation is timestamp-windowed, not "a tool was called
recently":

  - mark() records (wall-clock, file position) before the prompt is sent.
  - wait_for_tool() only accepts log entries at or after
    mark_time - skew_allowance (default 5s).
  - Entry timestamps are parsed from ISO-8601/RFC3339 prefixes when
    present (tracing's default fmt). Lines without a parseable timestamp
    fall back to position (after mark); the report notes which mode was
    used via the returned match's `correlation` attribute.
  - Clock skew: the allowance covers a slightly-behind server clock and
    buffered log flushes. Log buffering is handled by polling until the
    timeout; a `settle_s` pre-wait absorbs flush delay after the prompt.

Failure signals:
  SERVER_UNREACHABLE <url>              (exit 6)
  TOOL_NOT_CALLED <tool> :: <log tail>  (exit 7)
"""
from __future__ import annotations

import os
import re
import sys
import time
import json
import urllib.request
from datetime import datetime, timezone

from . import SERVER_UNREACHABLE, TOOL_NOT_CALLED, EXIT_SERVER, EXIT_TOOL

# ISO-8601/RFC3339 prefix, e.g. 2026-10-07T13:45:12.123456Z or
# 2026-10-07T13:45:12+00:00. tracing's fmt emits the former by default.
TS_RE = re.compile(
    r"(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:?\d{2})?)"
)


def _parse_ts(line: str) -> float | None:
    m = TS_RE.search(line)
    if not m:
        return None
    raw = m.group(1).replace("Z", "+00:00")
    try:
        return datetime.fromisoformat(raw).timestamp()
    except Exception:
        return None


class ProbeError(Exception):
    def __init__(self, signal: str, detail: str, exit_code: int):
        super().__init__(detail)
        self.signal = signal
        self.detail = detail
        self.exit_code = exit_code


class ToolMatch:
    """A correlated tool-call observation."""
    def __init__(self, tool: str, correlation: str, line: str):
        self.tool = tool
        self.correlation = correlation  # "timestamp" | "position"
        self.line = line


class ServerProbe:
    def __init__(self, cfg: dict, skew_allowance: float = 5.0):
        self.cfg = cfg
        self.base_url = cfg["server"]["base_url"].rstrip("/")
        self.log_file = cfg["server"].get("log_file") or ""
        self.skew_allowance = skew_allowance
        self._mark_time = 0.0
        self._pos = 0
        if self.log_file and os.path.exists(self.log_file):
            self._pos = os.path.getsize(self.log_file)

    def check_reachable(self) -> None:
        """Fail fast if the Maidan server isn't answering at all."""
        url = self.base_url + "/health/ready"
        try:
            with urllib.request.urlopen(url, timeout=10) as r:
                if r.status < 500:
                    return
        except Exception as e:
            raise ProbeError(
                SERVER_UNREACHABLE,
                f"{SERVER_UNREACHABLE} {self.base_url} ({e})",
                EXIT_SERVER,
            )
        raise ProbeError(
            SERVER_UNREACHABLE,
            f"{SERVER_UNREACHABLE} {self.base_url} (bad status)",
            EXIT_SERVER,
        )

    def _read_new(self) -> str:
        if not self.log_file or not os.path.exists(self.log_file):
            return ""
        with open(self.log_file, errors="replace") as f:
            f.seek(self._pos)
            data = f.read()
            self._pos = f.tell()
        return data

    def preflight(self) -> dict:
        """MCP preflight: verify the server exposes the expected tools.

        Calls tools/list on the MCP endpoint and returns the advertised
        tool names. Raises ProbeError(SERVER_UNREACHABLE) if the endpoint
        doesn't answer or doesn't speak MCP. Call once per run before
        suites; the result goes in the report so tool-not-called triage
        can distinguish "server missing the tool" from "model didn't
        call it".
        """
        url = self.base_url.rstrip("/") + "/mcp"
        payload = json.dumps({
            "jsonrpc": "2.0", "id": "harness-preflight",
            "method": "tools/list", "params": {},
        }).encode()
        req = urllib.request.Request(
            url, data=payload,
            headers={"Content-Type": "application/json",
                     "Accept": "application/json, text/event-stream"})
        try:
            with urllib.request.urlopen(req, timeout=15) as r:
                body = r.read().decode("utf-8", errors="replace")
        except Exception as e:
            raise ProbeError(
                SERVER_UNREACHABLE,
                f"{SERVER_UNREACHABLE} tools/list failed at {url} ({e})",
                EXIT_SERVER,
            )
        # SSE-wrapped or plain JSON: find the first JSON object with result.
        tools = []
        for line in body.splitlines():
            line = line.strip()
            if line.startswith("data:"):
                line = line[5:].strip()
            if not line.startswith("{"):
                continue
            try:
                obj = json.loads(line)
            except Exception:
                continue
            if not isinstance(obj, dict):
                continue
            # Malformed shapes (result not a dict, tools not a list of
            # dicts) are a server bug, not a crash: skip the line so the
            # preflight fails cleanly with "returned no tools" below.
            result = obj.get("result")
            if not isinstance(result, dict):
                continue
            raw_tools = result.get("tools") or []
            if not isinstance(raw_tools, list):
                continue
            for t in raw_tools:
                if not isinstance(t, dict):
                    continue
                name = t.get("name")
                if name:
                    tools.append(name)
            if tools:
                break
        if not tools:
            raise ProbeError(
                SERVER_UNREACHABLE,
                f"{SERVER_UNREACHABLE} tools/list at {url} returned no tools",
                EXIT_SERVER,
            )
        return {"tools": tools, "count": len(tools)}

    def mark(self) -> None:
        """Record the correlation window start: only tool calls at or after
        (now - skew_allowance) count."""
        self._mark_time = time.time()
        if self.log_file and os.path.exists(self.log_file):
            self._pos = os.path.getsize(self.log_file)

    def wait_for_tool(
        self,
        tool: str,
        timeout: float = 90.0,
        arg_fragment: str | None = None,
        settle_s: float = 2.0,
    ) -> ToolMatch:
        """Wait for a tools/call of `tool` correlated to the mark window.

        Raises ProbeError(TOOL_NOT_CALLED) with a log excerpt on timeout.
        If no log file is configured, raises immediately — a missing probe
        must never silently pass.
        """
        if not self.log_file:
            raise ProbeError(
                TOOL_NOT_CALLED,
                f"{TOOL_NOT_CALLED} {tool} :: no log_file configured; "
                "server-side assertion impossible",
                EXIT_TOOL,
            )
        time.sleep(settle_s)  # absorb log-buffer flush delay
        tool_re = re.compile(r'"name"\s*:\s*"' + re.escape(tool) + r'"')
        # The tool name also appears in tools/list responses enumerating
        # available tools. Only a tools/call counts as the tool being used.
        call_re = re.compile(r'tools/call')
        arg_re = re.compile(re.escape(arg_fragment)) if arg_fragment else None
        earliest = self._mark_time - self.skew_allowance
        deadline = time.time() + timeout
        buf = ""
        buf_base = self._pos  # file offset of buf[0]
        while time.time() < deadline:
            read_start = self._pos
            chunk = self._read_new()
            if not buf:
                buf_base = read_start
            buf += chunk
            # Scan line by line so timestamps gate per-entry. Track the
            # file offset of each line's end: on a match, _pos advances
            # to JUST PAST the matched line (not EOF), so a subsequent
            # wait_for_tool sees later calls. Advancing to EOF would make
            # in-order multi-tool sequences falsely fail.
            offset = buf_base
            for raw in buf.splitlines(keepends=True):
                line = raw.rstrip("\r\n")
                line_end = offset + len(raw)
                m = tool_re.search(line)
                if m and call_re.search(line):
                    ts = _parse_ts(line)
                    if ts is None or ts >= earliest:
                        window = line[max(0, m.start() - 2000):m.end() + 2000]
                        if arg_re is None or arg_re.search(window):
                            self._pos = line_end
                            mode = "timestamp" if ts is not None else "position"
                            return ToolMatch(tool, mode, line.strip()[:300])
                offset = line_end
            # Keep the buffer bounded; matches are line-local.
            # Trim to a line boundary and adjust buf_base so the file
            # offsets stay correct.
            if len(buf) > 200_000:
                cut = len(buf) - 50_000
                nl = buf.find("\n", cut)
                if nl != -1:
                    cut = nl + 1
                buf_base += cut
                buf = buf[cut:]
            time.sleep(1.0)
        tail = buf[-1500:] if buf.strip() else "(no new log lines)"
        raise ProbeError(
            TOOL_NOT_CALLED,
            f"{TOOL_NOT_CALLED} {tool} :: {tail}",
            EXIT_TOOL,
        )


def die(e: ProbeError) -> "None":
    print(e.detail, file=sys.stderr)
    sys.exit(e.exit_code)
