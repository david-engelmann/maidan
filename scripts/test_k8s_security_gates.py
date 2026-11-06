#!/usr/bin/env python3
"""Contract tests for k8s/helm security hardening."""

from __future__ import annotations

import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class K8sSecurityGate(unittest.TestCase):
    def test_repo_manifests_pass(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-k8s-security.sh")],
            cwd=ROOT,
            capture_output=True,
            text=True,
            timeout=20,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_missing_egress_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            # Minimal fake tree: copy script + strip egress from network policy
            (root / "scripts").mkdir()
            (root / "scripts/check-k8s-security.sh").write_text(
                (ROOT / "scripts/check-k8s-security.sh").read_text()
            )
            for rel in [
                "k8s/base/maidan-server-deployment.yaml",
                "k8s/base/postgres-statefulset.yaml",
                "k8s/base/minio-statefulset.yaml",
                "k8s/base/network-policy.yaml",
                "helm/maidan/values-prod.yaml",
                "helm/maidan/templates/deployment.yaml",
            ]:
                src = ROOT / rel
                dst = root / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                dst.write_text(src.read_text())
            np = root / "k8s/base/network-policy.yaml"
            text = np.read_text().replace("    - Egress\n", "").replace("  egress:\n", "  # egress:\n")
            # stronger: write ingress-only policy
            np.write_text(
                "apiVersion: networking.k8s.io/v1\nkind: NetworkPolicy\nmetadata: {name: x}\n"
                "spec:\n  podSelector: {}\n  policyTypes: [Ingress]\n  ingress: []\n"
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-k8s-security.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("Egress", r.stderr)

    def test_postgres_missing_drop_all_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "scripts/check-k8s-security.sh").write_text(
                (ROOT / "scripts/check-k8s-security.sh").read_text()
            )
            for rel in [
                "k8s/base/maidan-server-deployment.yaml",
                "k8s/base/postgres-statefulset.yaml",
                "k8s/base/minio-statefulset.yaml",
                "k8s/base/network-policy.yaml",
                "helm/maidan/values-prod.yaml",
                "helm/maidan/templates/deployment.yaml",
            ]:
                dst = root / rel
                dst.parent.mkdir(parents=True, exist_ok=True)
                dst.write_text((ROOT / rel).read_text())
            pg = root / "k8s/base/postgres-statefulset.yaml"
            pg.write_text(pg.read_text().replace('drop: ["ALL"]', 'drop: ["NET_RAW"]'))
            r = subprocess.run(
                ["bash", str(root / "scripts/check-k8s-security.sh")],
                cwd=root,
                capture_output=True,
                text=True,
                timeout=20,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("ALL", r.stderr)


class ManifestContents(unittest.TestCase):
    def test_statefulsets_are_non_root(self) -> None:
        for rel in (
            "k8s/base/postgres-statefulset.yaml",
            "k8s/base/minio-statefulset.yaml",
        ):
            text = (ROOT / rel).read_text()
            self.assertIn("runAsNonRoot: true", text)
            self.assertIn("allowPrivilegeEscalation: false", text)
            self.assertIn('drop: ["ALL"]', text)

    def test_network_policy_has_datastore_egress(self) -> None:
        text = (ROOT / "k8s/base/network-policy.yaml").read_text()
        self.assertIn("Egress", text)
        self.assertIn("5432", text)
        self.assertIn("9000", text)
        self.assertIn("component: postgres", text)
        self.assertIn("component: minio", text)

    def test_values_prod_enables_network_policy(self) -> None:
        text = (ROOT / "helm/maidan/values-prod.yaml").read_text()
        self.assertRegex(text, r"(?m)^networkPolicy:\n\s+enabled:\s+true")


if __name__ == "__main__":
    unittest.main()
