#!/usr/bin/env python3
"""Contract tests for deploy-dev / sdk-release / Alloy / Helm NP gates."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-deploy-dev-sdk-alloy-gates.sh"
SEED = (
    ".github/workflows/deploy-dev.yml",
    ".github/workflows/sdk-release.yml",
    "deploy/observability/alloy/config.alloy",
    "scripts/clean-machine-install.sh",
    "helm/maidan/templates/networkpolicy.yaml",
    "helm/maidan/values.yaml",
    "helm/maidan/values-ci.yaml",
    GATE,
)


def _run(cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(cwd / GATE)],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=20,
    )


def _seed(root: Path) -> None:
    for rel in SEED:
        src = ROOT / rel
        dst = root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)


class Gates(unittest.TestCase):
    def test_repo_passes(self) -> None:
        r = _run(ROOT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_deploy_dev_without_bootstrap_pin_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/deploy-dev.yml").read_text().replace(
                "MAIDAN_ENABLE_BOOTSTRAP=0\n", ""
            )
            (root / ".github/workflows/deploy-dev.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("BOOTSTRAP", r.stderr)

    def test_sdk_without_tag_shape_job_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/sdk-release.yml").read_text()
            text = text.replace("tag-shape:", "tag-shape-disabled:")
            text = text.replace("needs: [tag-shape]\n", "")
            (root / ".github/workflows/sdk-release.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("tag-shape", r.stderr)

    def test_alloy_without_token_redact_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "deploy/observability/alloy/config.alloy").read_text()
            text = text.replace('key    = "token"', 'key    = "token_removed"')
            (root / "deploy/observability/alloy/config.alloy").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("token", r.stderr)

    def test_networkpolicy_open_egress_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "helm/maidan/templates/networkpolicy.yaml").read_text()
            text = text.replace("allowAllEgress must be false", "allowAllEgress noted")
            text = text + "\n    - {}\n"
            (root / "helm/maidan/templates/networkpolicy.yaml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)

    def test_values_default_open_egress_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "helm/maidan/values.yaml").read_text().replace(
                "allowAllEgress: false", "allowAllEgress: true"
            )
            (root / "helm/maidan/values.yaml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("allowAllEgress", r.stderr)

    def test_clean_machine_without_path_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "scripts/clean-machine-install.sh").read_text().replace(
                "refusing path", "bad path"
            )
            (root / "scripts/clean-machine-install.sh").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("path traversal", r.stderr)


class Contents(unittest.TestCase):
    def test_values_ci_locks_network_policy(self) -> None:
        text = (ROOT / "helm/maidan/values-ci.yaml").read_text()
        self.assertIn("networkPolicy:", text)
        self.assertRegex(text, r"(?m)^\s*allowAllEgress:\s*false\s*$")

    def test_deploy_dev_pins_bootstrap_off(self) -> None:
        text = (ROOT / ".github/workflows/deploy-dev.yml").read_text()
        self.assertIn("MAIDAN_ENABLE_BOOTSTRAP=0", text)


if __name__ == "__main__":
    unittest.main()
