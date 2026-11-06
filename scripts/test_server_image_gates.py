#!/usr/bin/env python3
"""Contract tests for server / CLI / demo image and deploy workflow gates."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-server-image-gates.sh"
SEED = (
    "crates/maidan-server/Dockerfile",
    "docker/Dockerfile.cli",
    "deploy/cloudflare/demo/Dockerfile",
    ".github/workflows/deploy-demo.yml",
    ".github/workflows/deploy-prod.yml",
    ".github/workflows/clean-install.yml",
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


class ServerImageGate(unittest.TestCase):
    def test_repo_passes(self) -> None:
        r = _run(ROOT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_server_without_chmod_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "crates/maidan-server/Dockerfile").read_text().replace(
                "COPY --from=builder --chmod=0555", "COPY --from=builder"
            )
            (root / "crates/maidan-server/Dockerfile").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("chmod", r.stderr)

    def test_server_without_stopsignal_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "crates/maidan-server/Dockerfile").read_text().replace(
                "STOPSIGNAL SIGTERM\n", ""
            )
            (root / "crates/maidan-server/Dockerfile").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("STOPSIGNAL", r.stderr)

    def test_cli_without_license_label_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "docker/Dockerfile.cli").read_text().replace(
                'org.opencontainers.image.licenses="MIT" \\\n      ', ""
            )
            (root / "docker/Dockerfile.cli").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("license", r.stderr)

    def test_demo_without_entrypoint_chmod_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "deploy/cloudflare/demo/Dockerfile").read_text().replace(
                "COPY --chmod=0555 deploy/cloudflare/demo/entrypoint.sh",
                "COPY deploy/cloudflare/demo/entrypoint.sh",
            )
            (root / "deploy/cloudflare/demo/Dockerfile").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("entrypoint", r.stderr.lower())

    def test_demo_workflow_without_digest_shape_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/deploy-demo.yml").read_text()
            text = text.replace("sha256:[0-9a-f]{64}", "sha256:.*")
            (root / ".github/workflows/deploy-demo.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("digest", r.stderr.lower())

    def test_clean_install_without_semver_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/clean-install.yml").read_text().replace(
                "not a release tag", "bad tag shape"
            )
            (root / ".github/workflows/clean-install.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("non-vN.N.N", r.stderr)

    def test_prod_without_empty_tag_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/deploy-prod.yml").read_text().replace(
                '[ -n "${TAG:-}" ] || { echo "TAG is empty" >&2; exit 1; }\n          ',
                "",
            )
            (root / ".github/workflows/deploy-prod.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("empty TAG", r.stderr)


class Contents(unittest.TestCase):
    def test_demo_documents_digest_pin(self) -> None:
        text = (ROOT / "deploy/cloudflare/demo/Dockerfile").read_text()
        self.assertIn("@sha256:", text)
        self.assertIn("USER demo", text)

    def test_server_runtime_is_digest_pinned(self) -> None:
        text = (ROOT / "crates/maidan-server/Dockerfile").read_text()
        self.assertRegex(
            text,
            r"cgr\.dev/chainguard/glibc-dynamic@sha256:[0-9a-f]{64}",
        )


if __name__ == "__main__":
    unittest.main()
