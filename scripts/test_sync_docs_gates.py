#!/usr/bin/env python3
"""Tests for book/sync-docs-gates.py. Run: python3 scripts/test_sync_docs_gates.py"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATES = ROOT / "book" / "sync-docs-gates.py"

_SPEC = importlib.util.spec_from_file_location("sync_docs_gates", GATES)
mod = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = mod
_SPEC.loader.exec_module(mod)


class ValidateNames(unittest.TestCase):
    def test_page_ok(self):
        self.assertEqual(mod.validate_page_name("Capability Map"), "Capability Map")
        self.assertEqual(mod.staged_markdown_name("Capability Map"), "Capability-Map.md")

    def test_page_rejects_traversal(self):
        bad_names = [
            "../Architecture",
            "a/b",
            "a" + chr(92) + "b",
            "x" + chr(0) + "y",
            " spaced",
            "spaced ",
        ]
        for bad in bad_names:
            with self.subTest(bad=repr(bad)):
                with self.assertRaises(ValueError):
                    mod.validate_page_name(bad)

    def test_asset_ok_and_rejects_dotdot(self):
        self.assertEqual(mod.validate_asset_name("maidan-mark.svg"), "maidan-mark.svg")
        with self.assertRaises(ValueError):
            mod.validate_asset_name("../etc/passwd")
        with self.assertRaises(ValueError):
            mod.validate_asset_name(".hidden.svg")

    def test_cli_refuses_bad_name(self):
        proc = subprocess.run(
            [sys.executable, str(GATES), "check-name", "../nope"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 1)
        self.assertIn("sync-docs-gates", proc.stderr)


if __name__ == "__main__":
    unittest.main()
