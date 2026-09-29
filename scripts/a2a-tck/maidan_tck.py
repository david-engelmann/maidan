"""pytest plugin that runs the official A2A TCK against an authenticated Maidan.

Loaded with `-p maidan_tck` by scripts/a2a-tck.sh. It
  * adds `Authorization: Bearer $MAIDAN_TOKEN` to every TCK HTTP request and
    `authorization` metadata to every gRPC call (the TCK has no auth option;
    Maidan refuses anonymous A2A calls);
  * deselects the test ids listed in $A2A_TCK_EXCLUSIONS, each with a reason,
    and fails the run if an entry no longer matches any test;
  * fails the run if fewer than $A2A_TCK_MIN_PASSED tests pass, so a
    regression that turns a pass into a skip (the TCK skips a test whose
    set-up request fails) is caught like a failure.
"""

from __future__ import annotations

import os
from pathlib import Path

import httpx
import pytest

_TOKEN = os.environ.get("MAIDAN_TOKEN", "")


def _with_auth(init):
    def patched(self, *args, **kwargs):
        headers = httpx.Headers(kwargs.pop("headers", None))
        headers.setdefault("Authorization", f"Bearer {_TOKEN}")
        init(self, *args, headers=headers, **kwargs)

    return patched


if _TOKEN:
    httpx.Client.__init__ = _with_auth(httpx.Client.__init__)
    httpx.AsyncClient.__init__ = _with_auth(httpx.AsyncClient.__init__)

    from tck.transport.grpc_client import GrpcClient

    GrpcClient._METADATA = (*GrpcClient._METADATA, ("authorization", f"Bearer {_TOKEN}"))


def _exclusions() -> dict[str, str]:
    """`<test id prefix>  # <reason>` lines; blank lines and `#` comments skipped."""
    path = os.environ.get("A2A_TCK_EXCLUSIONS")
    out: dict[str, str] = {}
    if not path:
        return out
    for line in Path(path).read_text().splitlines():
        line = line.strip()
        if not line or line.startswith("#"):
            continue
        test_id, _, reason = line.partition("#")
        if not reason.strip():
            raise pytest.UsageError(f"exclusion without a reason: {line}")
        out[test_id.strip()] = reason.strip()
    return out


_EXCLUDED = _exclusions()
_unused = set(_EXCLUDED)
_passed = 0


def pytest_collection_modifyitems(config, items):
    keep, drop = [], []
    for item in items:
        match = next((p for p in _EXCLUDED if item.nodeid.startswith(p)), None)
        if match is None:
            keep.append(item)
        else:
            _unused.discard(match)
            drop.append(item)
    if drop:
        config.hook.pytest_deselected(items=drop)
        items[:] = keep


def pytest_runtest_logreport(report):
    global _passed
    if report.when == "call" and report.passed:
        _passed += 1


def pytest_sessionfinish(session, exitstatus):
    problems = []
    if _unused:
        problems.append(f"exclusions matching no test: {sorted(_unused)}")
    floor = int(os.environ.get("A2A_TCK_MIN_PASSED", "0"))
    if _passed < floor:
        problems.append(f"{_passed} tests passed; at least {floor} must")
    for problem in problems:
        print(f"\nA2A TCK: {problem}")
    if problems and session.exitstatus == 0:
        session.exitstatus = 1
