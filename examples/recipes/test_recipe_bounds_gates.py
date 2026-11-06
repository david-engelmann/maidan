#!/usr/bin/env python3
"""Contract tests for recipe agent env/command/role bounds (no network)."""

from __future__ import annotations

import importlib
import os
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from recipe_bounds import (
    validate_base_url,
    validate_channel_name,
    validate_id,
    validate_init_path,
    validate_lease_secs,
    validate_poll_secs,
    validate_role,
    validate_run_timeout_secs,
    validate_shell_command,
)


class RecipeBoundsGates(unittest.TestCase):
    def test_url_lease_poll_timeout(self) -> None:
        self.assertEqual(validate_base_url("http://maidan:8080"), "http://maidan:8080")
        with self.assertRaises(ValueError):
            validate_base_url("https://user:pass@maidan.world")
        self.assertEqual(validate_lease_secs(120), 120)
        with self.assertRaises(ValueError):
            validate_lease_secs(0)
        self.assertEqual(validate_poll_secs("3"), 3.0)
        with self.assertRaises(ValueError):
            validate_poll_secs("61")
        self.assertEqual(validate_run_timeout_secs("1800"), 1800)
        with self.assertRaises(ValueError):
            validate_run_timeout_secs("7201")

    def test_shell_command_bounds(self) -> None:
        self.assertEqual(validate_shell_command("", "DEPLOY_COMMAND"), "")
        self.assertEqual(validate_shell_command("echo ok", "DEPLOY_COMMAND"), "echo ok")
        with self.assertRaises(ValueError):
            validate_shell_command("x" * 5000, "DEPLOY_COMMAND")
        with self.assertRaises(ValueError):
            validate_shell_command("echo\nnope", "AGENT_COMMAND")
        with self.assertRaises(ValueError):
            validate_shell_command("echo CHANGE_ME", "AGENT_COMMAND")

    def test_role_channel_id_init(self) -> None:
        self.assertEqual(validate_role("coder", {"coder", "deployer"}), "coder")
        with self.assertRaises(ValueError):
            validate_role("root", {"coder", "deployer"})
        self.assertEqual(validate_channel_name("coding"), "coding")
        with self.assertRaises(ValueError):
            validate_channel_name("../x")
        self.assertEqual(validate_id("chan_1", "channel_id"), "chan_1")
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "init.txt"
            path.write_text("ok\n")
            self.assertEqual(validate_init_path(path), path)
            with self.assertRaises(ValueError):
                validate_init_path(Path(tmp) / "missing.txt")

    def test_agents_import_gated_defaults(self) -> None:
        env = {
            "LEASE_SECS": "120",
            "POLL_SECS": "3",
            "RUN_TIMEOUT_SECS": "60",
        }
        with mock.patch.dict(os.environ, env, clear=False):
            deploy = importlib.reload(importlib.import_module("deploy_agent"))
            coding = importlib.reload(importlib.import_module("coding_agent"))
            self.assertEqual(deploy.LEASE_SECS, 120)
            self.assertEqual(deploy.POLL_SECS, 3.0)
            self.assertEqual(coding.RUN_TIMEOUT_SECS, 60)
        with mock.patch.dict(os.environ, {"LEASE_SECS": "99999"}, clear=False):
            with self.assertRaises(ValueError):
                importlib.reload(importlib.import_module("deploy_agent"))

    def test_no_banned_internal_names(self) -> None:
        for name in (
            "recipe_bounds.py",
            "provision.py",
            "deploy_agent.py",
            "coding_agent.py",
        ):
            text = Path(__file__).with_name(name).read_text()
            self.assertNotRegex(
                text,
                r"Soundcheck|Pi world|Wax world|\bwax\b|bgv3|\bDawn\b",
            )


if __name__ == "__main__":
    unittest.main()
