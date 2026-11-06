#!/usr/bin/env python3
"""Contract tests for change_result.py input gates (no git, no network)."""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
MOD_PATH = HERE / "change_result.py"


def load():
    spec = importlib.util.spec_from_file_location("change_result", MOD_PATH)
    mod = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(mod)
    return mod


class ChangeResultGates(unittest.TestCase):
    @classmethod
    def setUpClass(cls) -> None:
        cls.m = load()

    def _args(self, **kwargs):
        base = dict(
            claim_lease_id="lease-abcdefgh",
            repo=None,
            base=None,
            max_bytes=self.m.DEFAULT_MAX_BYTES,
            out=None,
            summary_file=None,
            deliver_to_file=None,
            repo_dir=".",
            branch="feature/agent-demo",
            base_sha="0" * 40,
            head="HEAD",
            title=None,
            linear_issue=None,
        )
        base.update(kwargs)
        return SimpleNamespace(**base)

    def test_ok_defaults(self) -> None:
        self.m.validate_inputs(self._args())

    def test_claim_lease_bounds(self) -> None:
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(claim_lease_id="short"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(claim_lease_id="lease has space"))
        self.m.validate_inputs(self._args(claim_lease_id="a" * 128))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(claim_lease_id="a" * 129))

    def test_out_and_file_paths(self) -> None:
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(out="/tmp/out.json"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(out="../out.json"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(summary_file="~/secret.md"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(deliver_to_file="foo/../../etc/passwd"))
        self.m.validate_inputs(self._args(out="change-result.json"))

    def test_repo_and_base(self) -> None:
        self.m.validate_inputs(self._args(repo="acme/widgets", base="main"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(repo="not-a-repo"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(base="prod"))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(base="feat/../main"))

    def test_max_bytes(self) -> None:
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(max_bytes=10))
        with self.assertRaises(self.m.Stop):
            self.m.validate_inputs(self._args(max_bytes=self.m.DEFAULT_MAX_BYTES + 1))

    def test_skill_and_runbook_mention_gates(self) -> None:
        skill = (HERE / "SKILL.md").read_text()
        runbook = (HERE / "runbook.md").read_text()
        self.assertIn("refuse-closes", skill)
        self.assertIn("unsafe `--out`", skill)
        self.assertIn("refuse-closes", runbook)
        self.assertIn("prod` base", runbook)

    def test_build_calls_validate_before_branch(self) -> None:
        src = MOD_PATH.read_text()
        self.assertIn("validate_inputs(args)", src)
        self.assertLess(src.index("validate_inputs(args)"), src.index("CHANGE_BRANCH.match"))


if __name__ == "__main__":
    unittest.main()
