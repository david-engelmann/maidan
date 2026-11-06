#!/usr/bin/env python3
"""Contract tests for scripts/loadgen-guards.sh used by loadgen/replica-harness."""

from __future__ import annotations

import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GUARDS = ROOT / "scripts" / "loadgen-guards.sh"
UUIDS = (
    "aaaaaaaa-bbbb-cccc-dddd-111111111111|"
    "aaaaaaaa-bbbb-cccc-dddd-222222222222|"
    "aaaaaaaa-bbbb-cccc-dddd-333333333333|"
    "aaaaaaaa-bbbb-cccc-dddd-444444444444"
)


def _run(body: str, env: dict[str, str] | None = None) -> subprocess.CompletedProcess[str]:
    script = f"set -euo pipefail\nsource '{GUARDS}'\n{body}\n"
    full_env = None
    if env is not None:
        import os

        full_env = os.environ.copy()
        for key in (
            "MAIDAN_LOADGEN_URL",
            "MAIDAN_LOADGEN_BEARER",
            "MAIDAN_LOADGEN_IDS",
            "MAIDAN_LOADGEN_CONCURRENCY",
            "MAIDAN_LOADGEN_OPS",
            "MAIDAN_LOADGEN_DURATION_SECS",
        ):
            full_env.pop(key, None)
        full_env.update(env)
    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        timeout=10,
        env=full_env,
    )


class LoadgenGuards(unittest.TestCase):
    def test_scripts_source_guards(self) -> None:
        loadgen = (ROOT / "scripts" / "loadgen.sh").read_text()
        self.assertIn('source "$here/loadgen-guards.sh"', loadgen)
        self.assertIn("loadgen_validate_knobs", loadgen)
        self.assertNotIn("BEARER", loadgen.split("echo ", 1)[1].split("\n", 1)[0])
        replica = (ROOT / "scripts" / "replica-harness.sh").read_text()
        self.assertIn('source "$here/loadgen-guards.sh"', replica)
        self.assertIn("replica_validate_ports", replica)
        self.assertIn("set -euo pipefail", replica)

    def test_in_process_defaults_ok(self) -> None:
        r = _run("loadgen_validate_knobs", env={})
        self.assertEqual(r.returncode, 0, r.stderr)

    def test_url_loopback_and_https_ok(self) -> None:
        for url in (
            "http://127.0.0.1:8080",
            "http://localhost:8080",
            "http://[::1]:8080",
            "https://maidan.world",
        ):
            r = _run(
                "loadgen_validate_knobs",
                env={
                    "MAIDAN_LOADGEN_URL": url,
                    "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
                    "MAIDAN_LOADGEN_IDS": UUIDS,
                },
            )
            self.assertEqual(r.returncode, 0, f"{url}: {r.stderr}")

    def test_url_rejects(self) -> None:
        for url in (
            "http://example.com",
            "http://user:pass@127.0.0.1:8080",
            "ftp://127.0.0.1",
            "http://127.0.0.1:8080?x=1",
        ):
            r = _run(
                "loadgen_validate_knobs",
                env={
                    "MAIDAN_LOADGEN_URL": url,
                    "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
                    "MAIDAN_LOADGEN_IDS": UUIDS,
                },
            )
            self.assertNotEqual(r.returncode, 0, url)

    def test_bearer_and_ids_required_with_url(self) -> None:
        r = _run("loadgen_validate_knobs", env={"MAIDAN_LOADGEN_URL": "http://127.0.0.1:8080"})
        self.assertNotEqual(r.returncode, 0)
        r = _run(
            "loadgen_validate_knobs",
            env={
                "MAIDAN_LOADGEN_URL": "http://127.0.0.1:8080",
                "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
            },
        )
        self.assertNotEqual(r.returncode, 0)
        r = _run(
            "loadgen_validate_knobs",
            env={
                "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
            },
        )
        self.assertNotEqual(r.returncode, 0)

    def test_ids_shape(self) -> None:
        r = _run(
            "loadgen_validate_knobs",
            env={
                "MAIDAN_LOADGEN_URL": "http://127.0.0.1:8080",
                "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
                "MAIDAN_LOADGEN_IDS": "a|b|c",
            },
        )
        self.assertNotEqual(r.returncode, 0)
        r = _run(
            "loadgen_validate_knobs",
            env={
                "MAIDAN_LOADGEN_URL": "http://127.0.0.1:8080",
                "MAIDAN_LOADGEN_BEARER": "maid_abcdefghijklmnop",
                "MAIDAN_LOADGEN_IDS": UUIDS.replace("111111111111", "not-a-uuid-xxxxx"),
            },
        )
        self.assertNotEqual(r.returncode, 0)

    def test_knob_bounds(self) -> None:
        self.assertNotEqual(
            _run("loadgen_validate_knobs", env={"MAIDAN_LOADGEN_CONCURRENCY": "0"}).returncode,
            0,
        )
        self.assertNotEqual(
            _run("loadgen_validate_knobs", env={"MAIDAN_LOADGEN_CONCURRENCY": "257"}).returncode,
            0,
        )
        self.assertNotEqual(
            _run("loadgen_validate_knobs", env={"MAIDAN_LOADGEN_OPS": "0"}).returncode,
            0,
        )
        self.assertNotEqual(
            _run("loadgen_validate_knobs", env={"MAIDAN_LOADGEN_DURATION_SECS": "86401"}).returncode,
            0,
        )
        self.assertEqual(
            _run(
                "loadgen_validate_knobs",
                env={
                    "MAIDAN_LOADGEN_CONCURRENCY": "32",
                    "MAIDAN_LOADGEN_OPS": "50",
                    "MAIDAN_LOADGEN_DURATION_SECS": "60",
                },
            ).returncode,
            0,
        )

    def test_replica_ports(self) -> None:
        self.assertEqual(_run('replica_validate_ports "54321" "54322"').returncode, 0)
        self.assertNotEqual(_run('replica_validate_ports "80" "54322"').returncode, 0)
        self.assertNotEqual(_run('replica_validate_ports "54321" "54321"').returncode, 0)
        self.assertNotEqual(_run('replica_validate_ports "abc" "54322"').returncode, 0)


if __name__ == "__main__":
    unittest.main()
