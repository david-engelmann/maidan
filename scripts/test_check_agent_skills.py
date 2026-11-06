#!/usr/bin/env python3
"""Unit tests for scripts/check-agent-skills.py path and description gates."""

from __future__ import annotations

import importlib.util
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("check_agent_skills", ROOT / "check-agent-skills.py")
assert SPEC and SPEC.loader
mod = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = mod
SPEC.loader.exec_module(mod)


class SkillPathGates(unittest.TestCase):
    def test_skill_under_root_accepts_a_direct_child(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            skill = root / "maidan-claimant"
            skill.mkdir()
            self.assertTrue(mod.skill_under_root(skill, root))

    def test_skill_under_root_refuses_an_escape(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "skills"
            root.mkdir()
            outside = Path(tmp) / "outside"
            outside.mkdir()
            self.assertFalse(mod.skill_under_root(outside, root))
            nested = root / "group" / "maidan-claimant"
            nested.parent.mkdir()
            nested.mkdir()
            self.assertFalse(mod.skill_under_root(nested, root))

    def test_blank_description_is_refused_by_strip(self):
        meta = mod.frontmatter("---\nname: x\ndescription:    \n---\n\nbody\n")
        self.assertEqual(meta.get("description", "").strip(), "")


if __name__ == "__main__":
    unittest.main()
