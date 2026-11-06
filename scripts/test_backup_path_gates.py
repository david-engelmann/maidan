#!/usr/bin/env python3
"""Tests for backup_path_gates.py."""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
GATES = ROOT / "scripts" / "backup_path_gates.py"
_SPEC = importlib.util.spec_from_file_location("backup_path_gates", GATES)
mod = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = mod
_SPEC.loader.exec_module(mod)


class BackupPathGates(unittest.TestCase):
    def test_backup_dir_ok(self):
        self.assertEqual(mod.require_backup_dir("backups/2026"), "backups/2026")
        self.assertEqual(mod.require_backup_dir("/var/backups/maidan/"), "/var/backups/maidan")

    def test_backup_dir_rejects_roots_and_dotdot(self):
        for bad in ("/", "/tmp", "/tmp/", "..", "foo/../bar", "a\nb"):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    mod.require_backup_dir(bad)

    def test_artifact_root_requires_depth(self):
        self.assertEqual(
            mod.require_artifact_root("/var/lib/maidan/artifacts"),
            "/var/lib/maidan/artifacts",
        )
        self.assertEqual(mod.require_artifact_root("/tmp/maidan-arts"), "/tmp/maidan-arts")
        with self.assertRaises(ValueError):
            mod.require_artifact_root("/tmp")
        with self.assertRaises(ValueError):
            mod.require_artifact_root("relative/arts")
        with self.assertRaises(ValueError):
            mod.require_artifact_root("/etc/../tmp/x")

    def test_cli(self):
        good = subprocess.run(
            [sys.executable, str(GATES), "backup-dir", "backups/x"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(good.returncode, 0)
        bad = subprocess.run(
            [sys.executable, str(GATES), "artifact-root", "/tmp"],
            capture_output=True,
            text=True,
        )
        self.assertEqual(bad.returncode, 1)


if __name__ == "__main__":
    unittest.main()
