#!/usr/bin/env python3
"""Contract tests for compose/Docker privilege hardening."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class ComposeSecurityGate(unittest.TestCase):
    def test_repo_passes(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-compose-security.sh")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_missing_cap_drop_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "docker").mkdir()
            (root / "scripts/check-compose-security.sh").write_text(
                (ROOT / "scripts/check-compose-security.sh").read_text()
            )
            (root / "docker/Dockerfile.cli").write_text(
                (ROOT / "docker/Dockerfile.cli").read_text()
            )
            (root / "compose.yaml").write_text(
                "services:\n  maidan-server:\n    image: maidan-server:dev\n"
                "    security_opt: [no-new-privileges:true]\n"
            )
            (root / "compose.dev.yaml").write_text("services: {}\n")
            (root / "compose.pitr.yaml").write_text("services: {}\n")
            r = subprocess.run(
                ["bash", str(root / "scripts/check-compose-security.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("cap_drop", r.stderr)

    def test_cli_without_user_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "docker").mkdir()
            (root / "scripts/check-compose-security.sh").write_text(
                (ROOT / "scripts/check-compose-security.sh").read_text()
            )
            (root / "docker/Dockerfile.cli").write_text("FROM scratch\nCOPY maidan /maidan\n")
            (root / "compose.yaml").write_text(
                "services:\n  maidan-server:\n    image: x\n"
                "    security_opt: [no-new-privileges:true]\n    cap_drop: [ALL]\n"
            )
            (root / "compose.dev.yaml").write_text("services: {}\n")
            (root / "compose.pitr.yaml").write_text("services: {}\n")
            r = subprocess.run(
                ["bash", str(root / "scripts/check-compose-security.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("USER", r.stderr)


class ComposeContents(unittest.TestCase):
    def test_compose_yaml_hardens_scale_services(self) -> None:
        text = (ROOT / "compose.yaml").read_text()
        for name in ("maidan-server", "maidan-r1", "maidan-r2", "maidan-a", "maidan-b", "maidan-otlp"):
            self.assertIn(f"  {name}:", text)
        self.assertGreaterEqual(text.count("no-new-privileges:true"), 6)
        self.assertGreaterEqual(text.count("cap_drop:"), 6)

    def test_compose_dev_hardens_server(self) -> None:
        text = (ROOT / "compose.dev.yaml").read_text()
        self.assertIn("no-new-privileges:true", text)
        self.assertIn("cap_drop:", text)


if __name__ == "__main__":
    unittest.main()
