#!/usr/bin/env python3
"""Contract tests for Cloudflare wrangler fleet cost/security gates."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-wrangler-fleet-gates.sh"
SEED = (
    "deploy/cloudflare/wrangler.jsonc",
    "deploy/cloudflare/wrangler.prod.jsonc",
    "deploy/cloudflare/wrangler.demo.jsonc",
    "deploy/cloudflare/wrangler.docs.jsonc",
    "deploy/cloudflare/wrangler.swarm.jsonc",
    GATE,
)


def _run(cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(cwd / GATE)],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=20,
        env={"PYTHONDONTWRITEBYTECODE": "1"},
    )


def _seed(root: Path) -> None:
    for rel in SEED:
        src = ROOT / rel
        dst = root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)


class FleetGates(unittest.TestCase):
    def test_repo_passes(self) -> None:
        r = _run(ROOT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_workers_dev_true_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.jsonc"
            p.write_text(p.read_text().replace('"workers_dev": false', '"workers_dev": true', 1))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("workers_dev", r.stderr)

    def test_max_instances_raised_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.demo.jsonc"
            p.write_text(p.read_text().replace('"max_instances": 1', '"max_instances": 3', 1))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("max_instances", r.stderr)

    def test_hourly_cron_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.swarm.jsonc"
            p.write_text(p.read_text().replace('"23 9 * * *"', '"0 * * * *"', 1))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("cron", r.stderr.lower())

    def test_docs_with_main_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.docs.jsonc"
            text = p.read_text()
            text = text.replace(
                '"name": "maidan-docs",',
                '"name": "maidan-docs",\n  "main": "src/index.js",',
                1,
            )
            p.write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("static", r.stderr)

    def test_docs_with_triggers_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.docs.jsonc"
            text = p.read_text().rstrip()
            if text.endswith("}"):
                text = text[:-1] + ',\n  "triggers": { "crons": ["0 4 * * *"] }\n}\n'
            p.write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("static", r.stderr)

    def test_secret_shaped_var_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.prod.jsonc"
            p.write_text(
                p.read_text().replace(
                    '"S3_REGION": "auto",',
                    '"S3_REGION": "auto",\n    "MAIDAN_SESSION_SECRET": "leak",',
                    1,
                )
            )
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("secret-shaped", r.stderr)

    def test_non_maidan_route_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.jsonc"
            text = p.read_text()
            text = text.replace(
                '"pattern": "dev.maidan.world", "custom_domain": true',
                '"pattern": "evil.example.com", "custom_domain": true',
                1,
            )
            p.write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("maidan.world", r.stderr)

    def test_custom_domain_false_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.prod.jsonc"
            p.write_text(p.read_text().replace('"custom_domain": true', '"custom_domain": false', 1))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("custom_domain", r.stderr)

    def test_instance_type_not_basic_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.jsonc"
            p.write_text(p.read_text().replace('"instance_type": "basic"', '"instance_type": "standard"', 1))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("instance_type", r.stderr)


if __name__ == "__main__":
    unittest.main()
