#!/usr/bin/env python3
"""Stand-in for `capped`: the same interface, a wall-clock cap only.

Fail-closed on missing or out-of-range arguments so a miswired test cannot
sleep forever or run without a command.
"""
from __future__ import annotations

import subprocess
import sys

_MIN_SECS = 0.01
_MAX_SECS = 86400.0


def _usage(message: str) -> None:
    print(f"fake_capped: {message}", file=sys.stderr)
    print("usage: fake_capped --time <secs> [--mem-gb <n>] -- <command>...", file=sys.stderr)
    raise SystemExit(2)


def _parse(argv: list[str]) -> tuple[float, list[str]]:
    if "--" not in argv:
        _usage("missing '--' before the command")
    pre, command = argv[: argv.index("--")], argv[argv.index("--") + 1 :]
    if not command:
        _usage("command after '--' is empty")
    if "--time" not in pre:
        _usage("missing --time")
    try:
        secs = float(pre[pre.index("--time") + 1])
    except (ValueError, IndexError):
        _usage("--time needs a finite number of seconds")
    if secs != secs or secs in (float("inf"), float("-inf")):
        _usage("--time must be finite")
    if secs < _MIN_SECS or secs > _MAX_SECS:
        _usage(f"--time must be {_MIN_SECS}..{_MAX_SECS}")
    if "--mem-gb" in pre:
        try:
            mem = float(pre[pre.index("--mem-gb") + 1])
        except (ValueError, IndexError):
            _usage("--mem-gb needs a finite number")
        if mem != mem or mem in (float("inf"), float("-inf")) or mem <= 0 or mem > 1024:
            _usage("--mem-gb must be 0..1024 exclusive of 0 and non-finite")
    return secs, command


def main(argv: list[str] | None = None) -> int:
    secs, command = _parse(list(argv if argv is not None else sys.argv[1:]))
    child = subprocess.Popen(command)
    try:
        return int(child.wait(timeout=secs))
    except subprocess.TimeoutExpired:
        child.kill()
        return 124


if __name__ == "__main__":
    raise SystemExit(main())
