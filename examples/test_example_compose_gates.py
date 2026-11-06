#!/usr/bin/env python3
"""Fail-closed security and cost gates for example compose stacks and seat config."""

from __future__ import annotations

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent

OAUTH = ROOT / "oauth-dev" / "compose.yaml"
EMBED = ROOT / "embeddings" / "compose.ollama.yaml"
BASE = ROOT / "recipes" / "base.yaml"
CODING = ROOT / "recipes" / "coding-agent.yaml"
DEPLOY = ROOT / "recipes" / "deploy.yaml"
DOCKERFILE = ROOT / "recipes" / "Dockerfile.source"
SEATS = ROOT / "seat-runner" / "seats.example.toml"

_DIGEST = re.compile(r"@sha256:[0-9a-f]{64}")
_LOOPBACK_PORT = re.compile(r'"127\.0\.0\.1:\$\{?[A-Z0-9_:-]+\}?:?\d*"')
_UUID = re.compile(
    r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$"
)
_TOKEN_ENV = re.compile(r'^[A-Z][A-Z0-9_]{2,63}$')
_REPO = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")


def _banned(text: str) -> None:
    assert "Sound" + "check" not in text
    assert "Pi " + "world" not in text
    assert "Wax " + "world" not in text
    assert "bg" + "v3" not in text
    assert "D" + "awn" not in text


def _restart_policies(text: str) -> list[str]:
    return re.findall(r'restart:\s*["\']?([^\s"\'#]+)', text)


def _images(text: str) -> list[str]:
    return re.findall(r"(?m)^\s*image:\s*(\S+)", text)


class ExampleComposeGates(unittest.TestCase):
    def test_oauth_dev_is_loopback_pinned_and_not_always_on(self) -> None:
        text = OAUTH.read_text()
        self.assertIn("network_mode: host", text)
        self.assertIn("--http-host=127.0.0.1", text)
        self.assertIn('restart: "no"', text)
        images = _images(text)
        self.assertEqual(len(images), 1)
        self.assertTrue(_DIGEST.search(images[0]), images[0])
        self.assertRegex(
            text,
            r"(?m)^\s*KC_BOOTSTRAP_ADMIN_PASSWORD:\s*\$\{KC_ADMIN_PASSWORD:\?",
        )
        for line in text.splitlines():
            if "KC_BOOTSTRAP_ADMIN_PASSWORD:" in line and not line.lstrip().startswith("#"):
                self.assertIn("${KC_ADMIN_PASSWORD:", line, line)
                self.assertNotIn("PASSWORD: admin", line)
                self.assertNotIn("PASSWORD: password", line)
        self.assertNotIn("crons", text.lower())
        self.assertNotIn("unless-stopped", text)
        for policy in _restart_policies(text):
            self.assertEqual(policy, "no", policy)
        _banned(text)

    def test_embeddings_pin_cost_and_no_public_ports(self) -> None:
        text = EMBED.read_text()
        images = _images(text)
        self.assertGreaterEqual(len(images), 2)
        for img in images:
            self.assertTrue(img.startswith("ollama/ollama:"), img)
            # Prefer digest pin; tag-only is refused once a digest is present in-tree.
            if "@sha256:" in text:
                self.assertTrue(_DIGEST.search(img), img)
        self.assertNotRegex(text, r"(?m)^\s*ports:")
        self.assertIn('restart: "no"', text)
        for policy in _restart_policies(text):
            self.assertEqual(policy, "no", policy)
        self.assertIn("profiles:", text)
        self.assertIn("- full", text)
        self.assertIn("nomic-embed-text", text)
        self.assertIn("http://ollama:11434/v1/embeddings", text)
        self.assertNotIn("unless-stopped", text)
        _banned(text)

    def test_recipe_base_is_loopback_only_with_no_restart_loop(self) -> None:
        text = BASE.read_text()
        self.assertIn("127.0.0.1:${MAIDAN_HOST_PORT:-8080}:8080", text)
        self.assertNotRegex(text, r'(?m)^\s*-\s*"0\.0\.0\.0:')
        self.assertNotRegex(text, r'(?m)^\s*-\s*"\d+:\d+"')
        for policy in _restart_policies(text):
            self.assertEqual(policy, "no", policy)
        self.assertIn("MAIDAN_ALLOW_INSECURE_DEV_KEK", text)
        self.assertNotRegex(text, r"(?i)password\s*:\s*['\"]?[^\n$\{]")
        self.assertNotIn("unless-stopped", text)
        _banned(text)

    def test_recipe_agents_do_not_restart_forever(self) -> None:
        for path in (CODING, DEPLOY):
            with self.subTest(path=path.name):
                text = path.read_text()
                self.assertNotIn("unless-stopped", text)
                self.assertNotIn("restart: always", text)
                policies = _restart_policies(text)
                self.assertTrue(policies, path.name)
                for policy in policies:
                    self.assertEqual(policy, "no", f"{path.name}: {policy}")
                self.assertIn("MAIDAN_URL: http://maidan:8080", text)
                self.assertNotRegex(text, r"MAIDAN_TOKEN\s*:")
                self.assertNotRegex(text, r"(?i)bearer\s+maid")
                _banned(text)

    def test_dockerfile_runs_as_non_root_with_sigterm(self) -> None:
        text = DOCKERFILE.read_text()
        self.assertIn("USER maidan", text)
        self.assertIn("--uid 10001", text)
        self.assertIn("STOPSIGNAL SIGTERM", text)
        self.assertIn('ENTRYPOINT ["/usr/local/bin/maidan-server"]', text)
        self.assertNotIn("USER root", text)
        _banned(text)

    def test_seats_example_bounds_and_no_inline_token(self) -> None:
        text = SEATS.read_text()
        self.assertNotRegex(text, r"(?m)^\s*token\s*=")
        self.assertIn('token_env = "MAIDAN_TOKEN"', text)
        self.assertIn('url = "http://127.0.0.1:8080"', text)
        self.assertNotRegex(text, r'(?m)^\s*url\s*=\s*"http://(?!127\.0\.0\.1)')

        def _int(key: str) -> int:
            m = re.search(rf"(?m)^{re.escape(key)}\s*=\s*(\d+)", text)
            self.assertIsNotNone(m, key)
            return int(m.group(1))

        self.assertGreaterEqual(_int("seats"), 1)
        self.assertLessEqual(_int("seats"), 8)
        self.assertGreaterEqual(_int("lease_secs"), 60)
        self.assertLessEqual(_int("lease_secs"), 3600)
        self.assertGreaterEqual(_int("timeout_secs"), 1)
        self.assertLessEqual(_int("timeout_secs"), 120)
        self.assertGreaterEqual(_int("run_secs"), 60)
        self.assertLessEqual(_int("run_secs"), 3600)
        self.assertGreaterEqual(_int("run_mem_gb"), 1)
        self.assertLessEqual(_int("run_mem_gb"), 32)
        self.assertGreaterEqual(_int("preflight_secs"), 30)
        self.assertLessEqual(_int("preflight_secs"), 1800)
        self.assertGreaterEqual(_int("idle_secs"), 1)
        self.assertLessEqual(_int("idle_secs"), 3600)
        self.assertGreaterEqual(_int("kill_grace_secs"), 1)
        self.assertLessEqual(_int("kill_grace_secs"), 120)

        m = re.search(r'(?m)^workspace_id\s*=\s*"([^"]+)"', text)
        self.assertIsNotNone(m)
        self.assertRegex(m.group(1), _UUID)
        m = re.search(r'(?m)^token_env\s*=\s*"([^"]+)"', text)
        self.assertIsNotNone(m)
        self.assertRegex(m.group(1), _TOKEN_ENV)
        m = re.search(r'(?m)^repo\s*=\s*"([^"]+)"', text)
        self.assertIsNotNone(m)
        self.assertRegex(m.group(1), _REPO)
        self.assertNotRegex(text, r"Bearer\s+maid")
        _banned(text)


if __name__ == "__main__":
    unittest.main()
