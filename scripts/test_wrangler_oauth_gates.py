#!/usr/bin/env python3
"""Unit checks for wrangler host contract + OAuth/federation URL gates."""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class WranglerHosts(unittest.TestCase):
    def test_repo_configs_pass(self) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-wrangler-hosts.sh")],
            cwd=ROOT,
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_workers_dev_true_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "deploy/cloudflare").mkdir(parents=True)
            script = (ROOT / "scripts/check-wrangler-hosts.sh").read_text()
            (root / "scripts/check-wrangler-hosts.sh").write_text(script)
            (root / "deploy/cloudflare/wrangler.jsonc").write_text(
                '{\n  "name": "x",\n  "workers_dev": true,\n  "routes": [{"pattern": "dev.maidan.world", "custom_domain": true}]\n}\n'
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-wrangler-hosts.sh")],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("workers_dev", r.stderr)

    def test_origin_must_match_route(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "deploy/cloudflare").mkdir(parents=True)
            (root / "scripts/check-wrangler-hosts.sh").write_text(
                (ROOT / "scripts/check-wrangler-hosts.sh").read_text()
            )
            (root / "deploy/cloudflare/wrangler.jsonc").write_text(
                '{\n  "workers_dev": false,\n  "routes": [{"pattern": "dev.maidan.world", "custom_domain": true}],\n  "vars": {"MAIDAN_PUBLIC_ORIGIN": "https://evil.example"}\n}\n'
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-wrangler-hosts.sh")],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("MAIDAN_PUBLIC_ORIGIN", r.stderr)

    def test_non_maidan_host_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "scripts").mkdir()
            (root / "deploy/cloudflare").mkdir(parents=True)
            (root / "scripts/check-wrangler-hosts.sh").write_text(
                (ROOT / "scripts/check-wrangler-hosts.sh").read_text()
            )
            (root / "deploy/cloudflare/wrangler.jsonc").write_text(
                '{\n  "workers_dev": false,\n  "routes": [{"pattern": "evil.example", "custom_domain": true}]\n}\n'
            )
            r = subprocess.run(
                ["bash", str(root / "scripts/check-wrangler-hosts.sh")],
                cwd=root,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("maidan.world", r.stderr)


class ReleaseRecords(unittest.TestCase):
    def test_no_rg_dependency(self) -> None:
        src = (ROOT / "scripts/check-release-records.sh").read_text()
        self.assertNotRegex(src, r"\brg\b")
        self.assertIn("grep failed", src)


class UrlGuards(unittest.TestCase):
    def _reject(self, script: str, env: dict[str, str]) -> None:
        r = subprocess.run(
            ["bash", str(ROOT / script)],
            cwd=ROOT,
            env={**os.environ, **env},
            capture_output=True,
            text=True,
            timeout=8,
        )
        self.assertEqual(r.returncode, 2, f"{script}: {r.stdout}\n{r.stderr}")
        self.assertTrue(any(s in r.stderr for s in ("http(s)", "userinfo", "query or fragment")), r.stderr)

    def test_federation_rejects_file_url(self) -> None:
        self._reject(
            "scripts/federation-smoke.sh",
            {"MAIDAN_A_URL": "file:///etc/passwd", "MAIDAN_B_URL": "http://127.0.0.1:8081"},
        )

    def test_federation_pull_rejects_userinfo(self) -> None:
        self._reject(
            "scripts/federation-pull-smoke.sh",
            {
                "MAIDAN_A_URL": "https://user:pass@example.com",
                "MAIDAN_B_URL": "http://127.0.0.1:8081",
            },
        )

    def test_mcp_oauth_rejects_query(self) -> None:
        self._reject(
            "scripts/mcp-oauth-smoke.sh",
            {"MAIDAN_URL": "http://127.0.0.1:8080?x=1"},
        )

    def test_keycloak_rejects_fragment(self) -> None:
        env = {
            "MAIDAN_URL": "http://127.0.0.1:8080#x",
            "MAIDAN_WORKSPACE": "00000000-0000-0000-0000-000000000001",
            "OIDC_ISSUER": "http://127.0.0.1:9999/realms/x",
            "OIDC_CLIENT_ID": "x",
            "OIDC_USERNAME": "u",
            "OIDC_PASSWORD": "p",
        }
        self._reject("scripts/keycloak-oidc-smoke.sh", env)

    def test_oauth_dev_provider_rejects_bad_scheme(self) -> None:
        self._reject(
            "scripts/oauth-dev-provider.sh",
            {"MAIDAN_URL": "ftp://example.com"},
        )


class HelmTemplateSmoke(unittest.TestCase):
    def test_ci_refuses_missing_helm(self) -> None:
        src = (ROOT / "scripts/helm-template-smoke.sh").read_text()
        self.assertIn("refusing to skip template smoke under CI", src)
        # Soft skip still exists for local runs.
        self.assertIn("skipping template smoke", src)


if __name__ == "__main__":
    unittest.main()
