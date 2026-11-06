#!/usr/bin/env python3
"""Contract tests for recipe HTTP URL/token/path/lease gates (no network)."""

from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from maidan_http import (
    Maidan,
    claim_next,
    from_creds,
    validate_base_url,
    validate_creds_path,
    validate_id,
    validate_lease_secs,
    validate_rest_path,
    validate_timeout,
    validate_token,
    validate_tool_name,
)


class RecipeHttpGates(unittest.TestCase):
    def test_base_url_accepts_compose_and_https(self) -> None:
        self.assertEqual(validate_base_url("http://maidan:8080"), "http://maidan:8080")
        self.assertEqual(validate_base_url("https://maidan.world/"), "https://maidan.world")

    def test_base_url_refuses_credentials_and_schemes(self) -> None:
        with self.assertRaises(ValueError):
            validate_base_url("https://user:pass@maidan.world")
        with self.assertRaises(ValueError):
            validate_base_url("file:///etc/passwd")
        with self.assertRaises(ValueError):
            validate_base_url("http://maidan:99999")
        with self.assertRaises(ValueError):
            validate_base_url("http://maidan/../secret")

    def test_token_refuses_placeholder_and_whitespace(self) -> None:
        self.assertEqual(validate_token("m_live_abc.def~ghi"), "m_live_abc.def~ghi")
        with self.assertRaises(ValueError):
            validate_token("CHANGE_ME")
        with self.assertRaises(ValueError):
            validate_token("has space")
        with self.assertRaises(ValueError):
            validate_token("")

    def test_path_lease_tool_timeout(self) -> None:
        self.assertEqual(validate_rest_path("/threads/abc/claim"), "/threads/abc/claim")
        with self.assertRaises(ValueError):
            validate_rest_path("../etc/passwd")
        with self.assertRaises(ValueError):
            validate_rest_path("//evil")
        with self.assertRaises(ValueError):
            validate_rest_path("/threads/%2e%2e/%2e%2e/admin")
        with self.assertRaises(ValueError):
            validate_rest_path("/threads/%2E%2E/secret")
        self.assertEqual(validate_lease_secs(120), 120)
        with self.assertRaises(ValueError):
            validate_lease_secs(0)
        with self.assertRaises(ValueError):
            validate_lease_secs(3601)
        self.assertEqual(validate_tool_name("get_approval_gate"), "get_approval_gate")
        with self.assertRaises(ValueError):
            validate_tool_name("../tool")
        self.assertEqual(validate_timeout(60), 60.0)
        with self.assertRaises(ValueError):
            validate_timeout(0.5)
        self.assertEqual(validate_id("chan_1", "channel_id"), "chan_1")
        with self.assertRaises(ValueError):
            validate_id("../x", "channel_id")

    def test_maidan_init_and_call_path_gate(self) -> None:
        client = Maidan("http://maidan:8080", "tok_abc123")
        self.assertEqual(client.base_url, "http://maidan:8080")
        self.assertEqual(client.timeout, 60.0)
        with self.assertRaises(ValueError):
            Maidan("ftp://maidan", "tok_abc123")
        with self.assertRaises(ValueError):
            client.call("GET", "../threads")

    def test_from_creds_and_claim_next_gates(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "agent.json"
            path.write_text(json.dumps({"url": "http://maidan:8080", "token": "tok_abc123"}))
            client, creds = from_creds(str(path))
            self.assertEqual(client.base_url, "http://maidan:8080")
            self.assertEqual(creds["token"], "tok_abc123")
            with self.assertRaises(ValueError):
                validate_creds_path(str(Path(tmp) / "missing.json"))
            with self.assertRaises(ValueError):
                from_creds(str(Path(tmp) / ".." / ".." / "etc" / "passwd"))
        with mock.patch.object(Maidan, "call", return_value=None) as call:
            client = Maidan("http://maidan:8080", "tok_abc123")
            self.assertIsNone(claim_next(client, "channel_1", 30))
            call.assert_called_once()
            with self.assertRaises(ValueError):
                claim_next(client, "../bad", 30)

    def test_no_banned_internal_names(self) -> None:
        text = Path(__file__).with_name("maidan_http.py").read_text()
        self.assertNotRegex(text, r"Soundcheck|Pi world|Wax world|\bwax\b|bgv3|\bDawn\b")


if __name__ == "__main__":
    unittest.main()
