#!/usr/bin/env python3
"""Contract tests for scripts/demo-guards.sh used by demo-board/handoff."""

from __future__ import annotations

import os
import subprocess
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GUARDS = ROOT / "scripts" / "demo-guards.sh"


def _run(body: str) -> subprocess.CompletedProcess[str]:
    script = f"set -euo pipefail\nsource '{GUARDS}'\n{body}\n"
    return subprocess.run(
        ["bash", "-c", script],
        capture_output=True,
        text=True,
        timeout=10,
    )


class DemoGuards(unittest.TestCase):
    def test_board_and_handoff_source_guards(self) -> None:
        for rel in ("scripts/demo-board.sh", "scripts/demo-handoff.sh"):
            text = (ROOT / rel).read_text()
            self.assertIn('source "$here/demo-guards.sh"', text)
            self.assertIn("demo_validate_base", text)
            self.assertIn("demo_validate_rest", text)
            self.assertIn("demo_validate_mcp_tool", text)

    def test_loopback_http_and_https_ok(self) -> None:
        for url in (
            "http://127.0.0.1:8080",
            "http://localhost:8080",
            "http://[::1]:8080",
            "https://maidan.world",
        ):
            r = _run(f'demo_validate_base "{url}"')
            self.assertEqual(r.returncode, 0, r.stderr)

    def test_plain_http_remote_and_userinfo_fail(self) -> None:
        for url in (
            "http://example.com",
            "http://user:pass@127.0.0.1:8080",
            "https://user:pass@maidan.world",
            "ftp://127.0.0.1",
            "http://127.0.0.1:8080?x=1",
        ):
            r = _run(f'demo_validate_base "{url}"')
            self.assertNotEqual(r.returncode, 0, url)

    def test_workspace_uuid(self) -> None:
        r = _run('demo_validate_workspace "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee"')
        self.assertEqual(r.returncode, 0, r.stderr)
        r = _run('demo_validate_workspace "not-a-uuid"')
        self.assertNotEqual(r.returncode, 0)

    def test_token_bounds(self) -> None:
        r = _run('demo_validate_token "maid_abcdefghijklmnop"')
        self.assertEqual(r.returncode, 0, r.stderr)
        r = _run('demo_validate_token "short"')
        self.assertNotEqual(r.returncode, 0)
        r = _run('demo_validate_token "maid_abc def"')
        self.assertNotEqual(r.returncode, 0)

    def test_pause_and_ttl(self) -> None:
        self.assertEqual(_run('demo_validate_pause "0"').returncode, 0)
        self.assertEqual(_run('demo_validate_pause "60"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_pause "61"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_pause "1.5"').returncode, 0)
        self.assertEqual(_run('demo_validate_ttl ""').returncode, 0)
        self.assertEqual(_run('demo_validate_ttl "600"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_ttl "30"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_ttl "86401"').returncode, 0)

    def test_rest_method_and_path(self) -> None:
        self.assertEqual(
            _run(
                'demo_validate_rest POST "/workspaces/aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee/members"'
            ).returncode,
            0,
        )
        self.assertEqual(_run('demo_validate_rest GET "/threads/abc/context"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_rest TRACE "/threads/abc"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_rest GET "/admin/../etc/passwd"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_rest GET "/secret"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_rest GET "/threads/x?x=1"').returncode, 0)

    def test_mcp_tool_name(self) -> None:
        self.assertEqual(_run('demo_validate_mcp_tool "claim_next_thread"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_mcp_tool "../evil"').returncode, 0)
        self.assertNotEqual(_run('demo_validate_mcp_tool "tools/call"').returncode, 0)

    def test_board_refuses_bad_pause_before_curl(self) -> None:
        env = os.environ.copy()
        env.update(
            {
                "DEMO_PAUSE": "999",
                "MAIDAN_URL": "http://127.0.0.1:8080",
                "MAIDAN_TOKEN": "maid_abcdefghijklmnop",
                "MAIDAN_WORKSPACE": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            }
        )
        r = subprocess.run(
            ["bash", str(ROOT / "scripts" / "demo-board.sh")],
            capture_output=True,
            text=True,
            timeout=10,
            env=env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("DEMO_PAUSE", r.stderr)

    def test_handoff_refuses_userinfo_url(self) -> None:
        env = os.environ.copy()
        env.update(
            {
                "MAIDAN_URL": "https://user:pass@maidan.world",
                "MAIDAN_TOKEN": "maid_abcdefghijklmnop",
                "MAIDAN_WORKSPACE": "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            }
        )
        r = subprocess.run(
            ["bash", str(ROOT / "scripts" / "demo-handoff.sh")],
            capture_output=True,
            text=True,
            timeout=10,
            env=env,
        )
        self.assertNotEqual(r.returncode, 0)
        self.assertIn("userinfo", r.stderr)


if __name__ == "__main__":
    unittest.main()
