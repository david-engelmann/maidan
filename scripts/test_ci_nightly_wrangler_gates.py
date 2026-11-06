#!/usr/bin/env python3
"""Contract tests for CI/nightly/darwin credential pins and wrangler cron gates."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-ci-nightly-wrangler-gates.sh"
SEED = (
    ".github/workflows/ci.yml",
    ".github/workflows/nightly.yml",
    ".github/workflows/release-darwin-x86.yml",
    "deploy/cloudflare/wrangler.jsonc",
    "deploy/cloudflare/wrangler.prod.jsonc",
    "scripts/sdk-test.sh",
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

    def test_ci_checkout_without_persist_false_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/ci.yml").read_text()
            text = text.replace("persist-credentials: false\n", "", 1)
            (root / ".github/workflows/ci.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("persist-credentials", r.stderr)

    def test_nightly_checkout_without_persist_false_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/nightly.yml").read_text()
            text = text.replace("persist-credentials: false\n", "", 1)
            (root / ".github/workflows/nightly.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("persist-credentials", r.stderr)

    def test_darwin_without_tag_shape_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/release-darwin-x86.yml").read_text()
            text = text.replace("not a release tag", "bad tag")
            text = text.replace("release tag shape", "tag note")
            (root / ".github/workflows/release-darwin-x86.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)

    def test_wrangler_hourly_cron_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.jsonc"
            text = p.read_text()
            text = re_sub_cron(text, "0 * * * *")
            p.write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)

    def test_wrangler_prod_multi_hour_cron_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            p = root / "deploy/cloudflare/wrangler.prod.jsonc"
            text = p.read_text()
            text = re_sub_cron(text, "0 0,12 * * *")
            p.write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)

    def test_ci_without_contents_read_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/ci.yml").read_text()
            text = text.replace("contents: read", "contents: write", 1)
            (root / ".github/workflows/ci.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("contents: read", r.stderr)


    def test_sdk_test_without_port_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "scripts/sdk-test.sh").read_text().replace(
                "refusing out-of-range port", "bad port"
            )
            (root / "scripts/sdk-test.sh").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("port", r.stderr)


def re_sub_cron(text: str, new_expr: str) -> str:
    import re
    return re.sub(
        r'("crons"\s*:\s*\[\s*")[^"]+("\s*\])',
        rf'\g<1>{new_expr}\2',
        text,
        count=1,
    )


if __name__ == "__main__":
    unittest.main()
