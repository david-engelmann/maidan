#!/usr/bin/env python3
"""Tests for eval-land-gate-advisor input gates."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "eval_land_gate_advisor", Path(__file__).with_name("eval-land-gate-advisor.py")
)
mod = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = mod
_SPEC.loader.exec_module(mod)


class AdvisorGates(unittest.TestCase):
    def test_url_requires_http_host_without_userinfo(self):
        self.assertEqual(mod.require_advisor_url("https://maidan.world/"), "https://maidan.world")
        with self.assertRaises(ValueError):
            mod.require_advisor_url("file:///etc/passwd")
        with self.assertRaises(ValueError):
            mod.require_advisor_url("https://user:pass@maidan.world/")
        with self.assertRaises(ValueError):
            mod.require_advisor_url("https://maidan.world/path#frag")

    def test_thread_id_charset(self):
        self.assertEqual(mod.require_thread_id("thr_1"), "thr_1")
        with self.assertRaises(ValueError):
            mod.require_thread_id("../x")
        with self.assertRaises(ValueError):
            mod.require_thread_id("a b")
        with self.assertRaises(ValueError):
            mod.require_thread_id("")

    def test_load_cases_bounds(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "cases.jsonl"
            path.write_text(
                '{"state": {}, "actual": "green"}\n{"state": {}, "actual": "red"}\n',
                encoding="utf-8",
            )
            cases = mod.load_cases(path)
            self.assertEqual(len(cases), 2)
            huge = Path(tmp) / "huge.jsonl"
            huge.write_bytes(b"x" * (mod.MAX_CASES_BYTES + 1))
            with self.assertRaisesRegex(ValueError, "exceeds"):
                mod.load_cases(huge)


if __name__ == "__main__":
    unittest.main()
