#!/usr/bin/env python3
"""Fail-closed gates for claimant prompts and seat-runner harness fakes."""

from __future__ import annotations

import importlib.util
import json
import os
import re
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
SEAT = ROOT / "seat-runner"
PROMPTS = (
    SEAT / "prompt.md",
    SEAT / "review-prompt.md",
    ROOT / "cursor-agent" / "prompt.md",
    ROOT / "codex" / "prompt.md",
)
FAKE_CAPPED = SEAT / "tests" / "fake_capped.py"
FAKE_HARNESS = SEAT / "tests" / "fake_harness.py"

_REQUIRED = (
    "release_claim",
    "untrusted input",
    "Never print, paste or commit the seat token",
)
_FORBIDDEN = (
    "Sound" + "check",
    "Pi " + "world",
    "Wax " + "world",
    "bg" + "v3",
    "D" + "awn",
)


def _load(path: Path, name: str):
    spec = importlib.util.spec_from_file_location(name, path)
    assert spec and spec.loader
    mod = importlib.util.module_from_spec(spec)
    sys.modules[name] = mod
    spec.loader.exec_module(mod)
    return mod


class ClaimantPromptGates(unittest.TestCase):
    def test_prompts_share_security_rules(self) -> None:
        for path in PROMPTS:
            with self.subTest(path=path.name):
                text = path.read_text()
                for needle in ("untrusted input", "Never print, paste or commit the seat token"):
                    self.assertIn(needle, text, path.name)
                self.assertTrue(
                    "release_claim" in text or "start review, release" in text,
                    path.name,
                )
                self.assertTrue(
                    "Never force-push" in text or "Never push" in text,
                    path.name,
                )
                self.assertRegex(text, r"(?i)never.*(merge|mark)")
                self.assertIn("approve your own gate", text)
                self.assertIn("close the thread", text)
                if path.name != "review-prompt.md":
                    self.assertIn("one task", text.lower())
                self.assertNotRegex(text, r"lease_secs:\s*(?:[4-9]\d{3,}|\d{5,})")
                self.assertNotRegex(text, r"Bearer\s+maid")
                self.assertNotRegex(text, r"(?i)token\s*=\s*maid")
                for banned in _FORBIDDEN:
                    self.assertNotIn(banned, text)

    def test_report_usage_names_a_known_harness(self) -> None:
        for path in PROMPTS:
            text = path.read_text()
            self.assertRegex(
                text,
                r'"harness":\s*"(codex|cursor)"|`"harness":\s*"cursor"`',
                path.name,
            )


class FakeCappedGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.mod = _load(FAKE_CAPPED, "fake_capped_under_test")

    def test_happy_path_runs_command(self) -> None:
        rc = self.mod.main(["--time", "2", "--", sys.executable, "-c", "raise SystemExit(7)"])
        self.assertEqual(rc, 7)

    def test_timeout_returns_124(self) -> None:
        rc = self.mod.main(["--time", "0.05", "--", sys.executable, "-c", "import time; time.sleep(2)"])
        self.assertEqual(rc, 124)

    def test_refuses_missing_separator_and_bad_time(self) -> None:
        for argv in (
            ["--time", "1", "echo", "hi"],
            ["--time", "1", "--"],
            ["--", "true"],
            ["--time", "nan", "--", "true"],
            ["--time", "0", "--", "true"],
            ["--time", "999999", "--", "true"],
            ["--time", "1", "--mem-gb", "-1", "--", "true"],
        ):
            with self.subTest(argv=argv):
                with self.assertRaises(SystemExit) as ctx:
                    self.mod.main(argv)
                self.assertEqual(ctx.exception.code, 2)


class FakeHarnessGates(unittest.TestCase):
    def test_refuses_out_of_range_sleep_and_traversal_log(self) -> None:
        env = os.environ.copy()
        env["FAKE_HARNESS_SECS"] = "999"
        env["FAKE_HARNESS_LOG"] = "/tmp/fake-harness-ok.log"
        bad = subprocess.run([sys.executable, str(FAKE_HARNESS)], env=env, capture_output=True, text=True)
        self.assertEqual(bad.returncode, 2)

        with tempfile.TemporaryDirectory() as tmp:
            env["FAKE_HARNESS_SECS"] = "0"
            env["FAKE_HARNESS_LOG"] = str(Path(tmp) / ".." / ".." / "etc" / "passwd")
            # Path with .. parts must fail even if it resolves
            env["FAKE_HARNESS_LOG"] = "../outside.log"
            bad2 = subprocess.run(
                [sys.executable, str(FAKE_HARNESS)],
                env=env,
                cwd=tmp,
                capture_output=True,
                text=True,
            )
            self.assertEqual(bad2.returncode, 2)

    def test_writes_argv_pid_and_token_presence(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            log = Path(tmp) / "harness.log"
            env = os.environ.copy()
            env["FAKE_HARNESS_SECS"] = "0"
            env["FAKE_HARNESS_LOG"] = str(log)
            env["MAIDAN_TOKEN"] = "maid_test_token"
            ok = subprocess.run(
                [sys.executable, str(FAKE_HARNESS), "--model", "x"],
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertEqual(ok.returncode, 0, ok.stderr)
            row = json.loads(log.read_text().splitlines()[0])
            self.assertEqual(row["argv"], ["--model", "x"])
            self.assertEqual(row["token"], "maid_test_token")
            self.assertIsInstance(row["pid"], int)
            # Never echoed on stdout/stderr.
            self.assertNotIn("maid_test_token", ok.stdout)
            self.assertNotIn("maid_test_token", ok.stderr)


if __name__ == "__main__":
    unittest.main()
