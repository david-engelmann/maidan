#!/usr/bin/env python3
"""Stand-in for `codex`: records its argv, pid and whether it got the seat
token, then sleeps FAKE_HARNESS_SECS. FAKE_HARNESS_IGNORE_TERM=1 makes it
ignore SIGTERM, like a harness that will not stop.

Fail-closed on an out-of-range sleep or an unsafe log path so a miswired test
cannot hang the suite or write outside the work tree.
"""
from __future__ import annotations

import json
import os
import signal
import sys
import time
from pathlib import Path

_MIN_SECS = 0.0
_MAX_SECS = 120.0


def _fail(message: str) -> None:
    print(f"fake_harness: {message}", file=sys.stderr)
    raise SystemExit(2)


def _sleep_secs() -> float:
    raw = os.environ.get("FAKE_HARNESS_SECS", "0")
    try:
        secs = float(raw)
    except ValueError:
        _fail("FAKE_HARNESS_SECS must be a number")
    if secs != secs or secs in (float("inf"), float("-inf")):
        _fail("FAKE_HARNESS_SECS must be finite")
    if secs < _MIN_SECS or secs > _MAX_SECS:
        _fail(f"FAKE_HARNESS_SECS must be {_MIN_SECS}..{_MAX_SECS}")
    return secs


def _log_path() -> Path:
    raw = os.environ.get("FAKE_HARNESS_LOG")
    if not raw:
        _fail("FAKE_HARNESS_LOG is required")
    if "\0" in raw or "\n" in raw or "\r" in raw:
        _fail("FAKE_HARNESS_LOG has invalid characters")
    path = Path(raw)
    if ".." in path.parts:
        _fail("FAKE_HARNESS_LOG must not traverse")
    if not path.is_absolute() and len(path.parts) > 32:
        _fail("FAKE_HARNESS_LOG is too deep")
    return path


def main() -> int:
    if os.environ.get("FAKE_HARNESS_IGNORE_TERM") == "1":
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
    log = _log_path()
    log.parent.mkdir(parents=True, exist_ok=True)
    # Keep the token value for existing seat-runner tests that assert pass_token
    # behaviour; never print it to stdout/stderr.
    with open(log, "a", encoding="utf-8") as f:
        f.write(
            json.dumps(
                {
                    "argv": sys.argv[1:],
                    "pid": os.getpid(),
                    "token": os.environ.get("MAIDAN_TOKEN"),
                }
            )
            + "\n"
        )
    time.sleep(_sleep_secs())
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
