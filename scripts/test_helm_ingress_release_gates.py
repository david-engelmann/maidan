#!/usr/bin/env python3
"""Contract tests for Helm ingress/release/docs/kind-smoke gates."""

from __future__ import annotations

import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-helm-ingress-release-gates.sh"
SEED = (
    "helm/maidan/templates/ingress.yaml",
    "helm/maidan/templates/deployment.yaml",
    "helm/maidan/templates/service.yaml",
    "helm/maidan/templates/poddisruptionbudget.yaml",
    ".github/workflows/release.yml",
    ".github/workflows/docs.yml",
    "scripts/helm-install-kind-smoke.sh",
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

    def test_ingress_without_tls_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "helm/maidan/templates/ingress.yaml").read_text().replace(
                "production ingress needs tls", "production ingress prefers tls"
            )
            (root / "helm/maidan/templates/ingress.yaml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("TLS", r.stderr)

    def test_deployment_without_automount_false_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "helm/maidan/templates/deployment.yaml").read_text().replace(
                "automountServiceAccountToken: false",
                "automountServiceAccountToken: true",
            )
            (root / "helm/maidan/templates/deployment.yaml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("automountServiceAccountToken", r.stderr)

    def test_service_without_prod_type_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            (root / "helm/maidan/templates/service.yaml").write_text(
                "apiVersion: v1\nkind: Service\n"
            )
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("NodePort", r.stderr)

    def test_docs_workflow_scope_pages_write_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/docs.yml").read_text()
            text = text.replace(
                "permissions:\n  contents: read\n\njobs:",
                "permissions:\n  contents: read\n  pages: write\n  id-token: write\n\njobs:",
                1,
            )
            (root / ".github/workflows/docs.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("pages: write", r.stderr)

    def test_release_without_tag_guard_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / ".github/workflows/release.yml").read_text().replace(
                "not a release tag", "bad tag shape"
            )
            (root / ".github/workflows/release.yml").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("vN.N.N", r.stderr)

    def test_smoke_without_port_bound_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "scripts/helm-install-kind-smoke.sh").read_text().replace(
                "port out of range", "port unusual"
            )
            (root / "scripts/helm-install-kind-smoke.sh").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("ports", r.stderr)


class Contents(unittest.TestCase):
    def test_ingress_requires_prefix_or_exact(self) -> None:
        text = (ROOT / "helm/maidan/templates/ingress.yaml").read_text()
        self.assertIn("Prefix", text)
        self.assertIn("Exact", text)

    def test_docs_deploy_job_has_pages_write(self) -> None:
        text = (ROOT / ".github/workflows/docs.yml").read_text()
        deploy = text.split("deploy:", 1)[1]
        self.assertIn("pages: write", deploy)


if __name__ == "__main__":
    unittest.main()
