#!/usr/bin/env python3
"""Unit checks for CI cost cron gate and smoke input bounds."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class CiCost(unittest.TestCase):
    def test_repo_schedules_pass(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-ci-cost.sh")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_hourly_workflow_cron_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / ".github/workflows").mkdir(parents=True)
            (root / "deploy/cloudflare").mkdir(parents=True)
            (root / "scripts/check-ci-cost.sh").write_text(
                (ROOT / "scripts/check-ci-cost.sh").read_text()
            )
            (root / ".github/workflows/bad.yml").write_text(
                "on:\n  schedule:\n    - cron: '0 * * * *'\njobs: {}\n"
            )
            (root / "deploy/cloudflare/wrangler.jsonc").write_text(
                '{ "triggers": { "crons": ["17 9 * * *"] } }\n'
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-ci-cost.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("hour", r.stderr)

    def test_step_minute_wrangler_cron_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / ".github/workflows").mkdir(parents=True)
            (root / "deploy/cloudflare").mkdir(parents=True)
            (root / "scripts/check-ci-cost.sh").write_text(
                (ROOT / "scripts/check-ci-cost.sh").read_text()
            )
            (root / ".github/workflows/ok.yml").write_text(
                "on:\n  schedule:\n    - cron: '17 3 * * *'\njobs: {}\n"
            )
            (root / "deploy/cloudflare/wrangler.jsonc").write_text(
                '{ "triggers": { "crons": ["*/15 * * * *"] } }\n'
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-ci-cost.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("minute", r.stderr)


class SmokePorts(unittest.TestCase):
    def _reject(self, script: str, env: dict[str, str], needle: str) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / script)],
            cwd=ROOT,
            env={**os.environ, **env},
            capture_output=True,
            text=True,
            timeout=8,
        )
        self.assertEqual(r.returncode, 2, f"{script}: {r.stderr}\n{r.stdout}")
        self.assertIn(needle, r.stderr)

    def test_otlp_rejects_bad_port(self) -> None:
        self._reject("scripts/otlp-smoke.sh", {"MAIDAN_OTLP_PORT": "nope"}, "integer port")

    def test_scale_rejects_out_of_range(self) -> None:
        self._reject("scripts/scale-out-smoke.sh", {"MAIDAN_SCALE_LB_PORT": "70000"}, "65535")

    def test_mcp_inspector_rejects_zero(self) -> None:
        self._reject("scripts/mcp-inspector.sh", {"MAIDAN_MCP_PORT": "0"}, "65535")

    def test_openapi_rejects_empty(self) -> None:
        self._reject("scripts/openapi-lint.sh", {"MAIDAN_OPENAPI_PORT": "nope"}, "integer port")


class ReleaseImageSmoke(unittest.TestCase):
    def test_refuses_latest(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/release-image-smoke.sh"), "latest"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=8,
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("immutable", r.stderr)

    def test_refuses_path_owner(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/release-image-smoke.sh"), "v1.0.0", "evil/../x"],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=8,
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("owner", r.stderr)


if __name__ == "__main__":
    unittest.main()
