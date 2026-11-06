#!/usr/bin/env python3
"""Fail-closed security contract for example MCP connectors and OAuth clients."""

from __future__ import annotations

import json
import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent

MCP_JSON = (
    ROOT / "claude-code" / "mcp.json",
    ROOT / "claude-desktop-mcp.json",
    ROOT / "cursor" / "mcp.json",
    ROOT / "cursor-mcp.json",
)
CONNECT = ROOT / "connect" / "clients.example.json"
GOOSE = ROOT / "goose" / "maidan-waiter.yaml"
CODEX = ROOT / "codex" / "config.toml"

_LOOPBACK_HTTP = re.compile(r"^http://127\.0\.0\.1(?::\d{1,5})?(?:/.*)?$")
_HOSTED_HTTPS = re.compile(r"^https://maidan\.world(?:/.*)?$")
_ENV_URL = re.compile(
    r"^\$\{MAIDAN_URL:-http://127\.0\.0\.1(?::\d{1,5})?\}(?:/.*)?$"
)
_TOKEN_OK = re.compile(
    r"^(?:Bearer\s+)?(?:\$\{(?:env:)?MAIDAN_TOKEN\}|\$\{AUTH_HEADER\}|REPLACE_WITH_[A-Z0-9_]+)$"
)
_KNOWN_SCOPES = frozenset({"workspace:read", "workspace:write"})
_MCP_REMOTE_PIN = re.compile(r"^mcp-remote@\d+\.\d+\.\d+$")


def _url_ok(url: str) -> bool:
    return bool(_LOOPBACK_HTTP.match(url) or _HOSTED_HTTPS.match(url) or _ENV_URL.match(url))


def _auth_ok(value: str) -> bool:
    return bool(_TOKEN_OK.match(value.strip()))


class McpConnectorGates(unittest.TestCase):
    def test_mcp_json_urls_and_tokens(self) -> None:
        for path in MCP_JSON:
            with self.subTest(path=path.name):
                data = json.loads(path.read_text())
                servers = data.get("mcpServers") or {}
                self.assertIn("maidan", servers)
                srv = servers["maidan"]
                if "url" in srv:
                    self.assertEqual(srv.get("type"), "http", path.name)
                    self.assertTrue(_url_ok(srv["url"]), srv["url"])
                    auth = (srv.get("headers") or {}).get("Authorization", "")
                    self.assertTrue(_auth_ok(auth), auth)
                    self.assertNotRegex(auth, r"Bearer\s+maid[_-]")
                if srv.get("command") == "npx":
                    args = srv.get("args") or []
                    pins = [a for a in args if a.startswith("mcp-remote@")]
                    self.assertEqual(len(pins), 1, path.name)
                    self.assertRegex(pins[0], _MCP_REMOTE_PIN)
                    self.assertTrue(any(_HOSTED_HTTPS.match(a) for a in args), args)
                    env = srv.get("env") or {}
                    self.assertTrue(any("REPLACE_WITH_" in v for v in env.values()), env)

    def test_no_plaintext_secrets_in_mcp_json(self) -> None:
        for path in MCP_JSON:
            text = path.read_text()
            for match in re.finditer(r"Bearer\s+(\S+)", text):
                token = match.group(1).strip('"')
                self.assertTrue(
                    _auth_ok("Bearer " + token) or token.startswith("REPLACE_WITH_"),
                    f"{path.name}: inline token {token!r}",
                )
                self.assertFalse(re.fullmatch(r"maid[_-][A-Za-z0-9._-]{8,}", token))
            self.assertNotIn('"token":', text.lower().replace(" ", ""))

    def test_connect_oauth_clients(self) -> None:
        clients = json.loads(CONNECT.read_text())
        self.assertTrue(clients)
        for client in clients:
            with self.subTest(client_id=client.get("client_id")):
                self.assertTrue(re.fullmatch(r"[a-z][a-z0-9-]{1,63}", client["client_id"]))
                for uri in client.get("redirect_uris") or []:
                    self.assertTrue(
                        uri.startswith("https://") or uri.startswith("REPLACE_WITH_"),
                        uri,
                    )
                    self.assertNotIn("*", uri)
                scopes = set(client.get("allowed_scopes") or [])
                self.assertTrue(scopes)
                self.assertTrue(scopes <= _KNOWN_SCOPES, scopes)
                if "client_secret_hash" in client:
                    h = client["client_secret_hash"]
                    self.assertTrue(
                        h.startswith("REPLACE_WITH_") or re.fullmatch(r"[0-9a-f]{64}", h),
                        h,
                    )

    def test_goose_waiter_uses_env_token_and_loopback(self) -> None:
        text = GOOSE.read_text()
        self.assertIn("uri: http://127.0.0.1:8080/mcp/streamable", text)
        self.assertIn("Authorization: Bearer ${MAIDAN_TOKEN}", text)
        self.assertIn("- MAIDAN_TOKEN", text)
        self.assertIn("timeout: 300", text)
        self.assertNotRegex(text, r"Bearer\s+maid[_-]")

    def test_codex_uses_env_var_not_inline_token(self) -> None:
        text = CODEX.read_text()
        self.assertIn('url = "http://127.0.0.1:8080/mcp/streamable"', text)
        self.assertIn('bearer_token_env_var = "MAIDAN_TOKEN"', text)
        self.assertNotRegex(text, r"bearer_token\s*=")
        # tool timeout bounded for claim waits
        m = re.search(r"tool_timeout_sec\s*=\s*(\d+)", text)
        self.assertIsNotNone(m)
        self.assertLessEqual(int(m.group(1)), 600)
        self.assertGreaterEqual(int(m.group(1)), 60)

    def test_no_banned_internal_names(self) -> None:
        paths = list(MCP_JSON) + [CONNECT, GOOSE, CODEX]
        for path in paths:
            text = path.read_text()
            self.assertNotIn("Sound" + "check", text)
            self.assertNotIn("Pi " + "world", text)
            self.assertNotIn("Wax " + "world", text)
            self.assertNotIn("bg" + "v3", text)
            self.assertNotIn("w" + "ax", text.lower())
            self.assertNotIn("D" + "awn", text)


if __name__ == "__main__":
    unittest.main()
