#!/usr/bin/env python3
"""Contract tests for example MCP client URL/token/channel/lease gates."""

from __future__ import annotations

import unittest
from pathlib import Path

from example_client_bounds import (
    validate_base_url,
    validate_channel_id,
    validate_lease_secs,
    validate_token,
)


class ExampleClientBounds(unittest.TestCase):
    def test_base_url(self) -> None:
        self.assertEqual(
            validate_base_url("http://127.0.0.1:8080"),
            "http://127.0.0.1:8080",
        )
        self.assertEqual(
            validate_base_url("https://maidan.world/"),
            "https://maidan.world",
        )
        with self.assertRaises(ValueError):
            validate_base_url("https://user:pass@maidan.world")
        with self.assertRaises(ValueError):
            validate_base_url("http://maidan.world")
        with self.assertRaises(ValueError):
            validate_base_url("file:///etc/passwd")

    def test_token_channel_lease(self) -> None:
        self.assertEqual(validate_token("maid_abc.def"), "maid_abc.def")
        with self.assertRaises(ValueError):
            validate_token("")
        with self.assertRaises(ValueError):
            validate_token("CHANGE_ME")
        self.assertEqual(validate_channel_id("chan_1"), "chan_1")
        with self.assertRaises(ValueError):
            validate_channel_id("../x")
        with self.assertRaises(ValueError):
            validate_channel_id("%2e%2e")
        self.assertEqual(validate_lease_secs(600), 600)
        with self.assertRaises(ValueError):
            validate_lease_secs(0)
        with self.assertRaises(ValueError):
            validate_lease_secs(3601)

    def test_clients_import_bounds(self) -> None:
        root = Path(__file__).resolve().parent
        for rel in (
            "openhands/maidan_claimant.py",
            "langchain_maidan.py",
            "autogen_maidan.py",
        ):
            text = (root / rel).read_text()
            self.assertIn("validate_base_url", text)
            self.assertIn("validate_token", text)
            self.assertNotRegex(
                text,
                r"Soundcheck|Pi world|Wax world|\bwax\b|bgv3|\bDawn\b",
            )


if __name__ == "__main__":
    unittest.main()
