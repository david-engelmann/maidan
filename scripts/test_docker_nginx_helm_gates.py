#!/usr/bin/env python3
"""Contract tests for docker nginx / DB / OTEL / Helm values.schema gates."""

from __future__ import annotations

import json
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATE = "scripts/check-docker-nginx-helm-gates.sh"


def _run(cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        ["bash", str(cwd / GATE)],
        cwd=cwd,
        capture_output=True,
        text=True,
        timeout=20,
    )


def _seed(root: Path) -> None:
    for rel in (
        "docker/nginx-scale.conf",
        "docker/Dockerfile.db",
        "docker/otel-collector-config.yaml",
        "deploy/cloudflare/demo/Dockerfile",
        "helm/maidan/values.schema.json",
        "helm/maidan/values.yaml",
        GATE,
    ):
        src = ROOT / rel
        dst = root / rel
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
    (root / "scripts").mkdir(exist_ok=True)


class DockerNginxHelmGate(unittest.TestCase):
    def test_repo_passes(self) -> None:
        r = _run(ROOT)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn("ok", r.stdout)

    def test_nginx_without_server_tokens_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            text = (root / "docker/nginx-scale.conf").read_text().replace(
                "server_tokens off;", "# server_tokens off;"
            )
            (root / "docker/nginx-scale.conf").write_text(text)
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("server_tokens", r.stderr)

    def test_db_without_initdb_wipe_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            (root / "docker/Dockerfile.db").write_text(
                "FROM pgvector/pgvector:pg16\n"
                "RUN mkdir -p /archive && chown postgres:postgres /archive && chmod 700 /archive\n"
            )
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("initdb", r.stderr)

    def test_otel_without_memory_limiter_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            (root / "docker/otel-collector-config.yaml").write_text(
                "receivers:\n  otlp:\n    protocols:\n      grpc:\n        endpoint: 0.0.0.0:4317\n"
                "exporters:\n  debug:\n    verbosity: detailed\n"
                "service:\n  pipelines:\n    traces:\n      receivers: [otlp]\n      exporters: [debug]\n"
            )
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("memory_limiter", r.stderr)

    def test_schema_without_nonroot_const_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            schema = json.loads((root / "helm/maidan/values.schema.json").read_text())
            schema["properties"]["podSecurityContext"]["properties"]["runAsNonRoot"] = {
                "type": "boolean"
            }
            (root / "helm/maidan/values.schema.json").write_text(json.dumps(schema))
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("runAsNonRoot", r.stderr)

    def test_demo_without_user_fails(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            _seed(root)
            (root / "deploy/cloudflare/demo/Dockerfile").write_text(
                "FROM debian:bookworm-slim\nCOPY x /x\n"
            )
            r = _run(root)
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("USER", r.stderr)


class Contents(unittest.TestCase):
    def test_nginx_has_security_headers(self) -> None:
        text = (ROOT / "docker/nginx-scale.conf").read_text()
        self.assertIn("X-Content-Type-Options", text)
        self.assertIn("client_max_body_size", text)
        self.assertIn("proxy_hide_header Server", text)

    def test_values_schema_matches_defaults_keys(self) -> None:
        schema = json.loads((ROOT / "helm/maidan/values.schema.json").read_text())
        self.assertIn("podSecurityContext", schema["required"])
        self.assertIn("resources", schema["required"])


if __name__ == "__main__":
    unittest.main()
