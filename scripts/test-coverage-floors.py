#!/usr/bin/env python3
"""Tests for coverage-floors.py. Run: python3 scripts/test-coverage-floors.py"""

import importlib.util
import sys
import unittest
from pathlib import Path

_SPEC = importlib.util.spec_from_file_location(
    "coverage_floors", Path(__file__).with_name("coverage-floors.py")
)
floors = importlib.util.module_from_spec(_SPEC)
sys.modules[_SPEC.name] = floors
_SPEC.loader.exec_module(floors)

LCOV = """\
SF:/home/runner/work/maidan/maidan/crates/maidan-auth/src/lib.rs
LF:10
LH:9
end_of_record
SF:/home/runner/work/maidan/maidan/crates/maidan-auth/src/token.rs
LF:10
LH:10
end_of_record
SF:/home/runner/work/maidan/maidan/crates/maidan-fsm/src/lib.rs
LF:4
LH:2
end_of_record
SF:/home/runner/.cargo/registry/src/some-dep/src/lib.rs
LF:100
LH:0
end_of_record
SF:/home/runner/work/maidan/maidan/crates/maidan-fsm/src/a2a_grpc/generated.serde.rs
LF:1000
LH:1
end_of_record
SF:/home/runner/work/maidan/maidan/crates/maidan-fsm/src/a2a_grpc/generated.rs
LF:500
LH:1
end_of_record
"""


def config(workspace, **crates):
    return {"workspace": {"lines": workspace}, "crates": crates}


class PerCrate(unittest.TestCase):
    def test_lines_are_summed_per_crate_and_foreign_and_generated_files_ignored(self):
        totals = floors.per_crate(LCOV)
        self.assertEqual(sorted(totals), ["maidan-auth", "maidan-fsm"])
        self.assertEqual((totals["maidan-auth"].covered, totals["maidan-auth"].found), (19, 20))
        self.assertEqual(totals["maidan-fsm"].percent, 50.0)


class Check(unittest.TestCase):
    def setUp(self):
        self.totals = floors.per_crate(LCOV)

    def test_every_floor_met_passes(self):
        _, failures = floors.check(
            self.totals, config(87.0, **{"maidan-auth": 95.0, "maidan-fsm": 50.0})
        )
        self.assertEqual(failures, [])

    def test_a_crate_under_its_floor_fails(self):
        _, failures = floors.check(
            self.totals, config(0.0, **{"maidan-auth": 95.0, "maidan-fsm": 50.5})
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("maidan-fsm", failures[0])

    def test_the_workspace_under_its_floor_fails(self):
        _, failures = floors.check(
            self.totals, config(88.0, **{"maidan-auth": 0.0, "maidan-fsm": 0.0})
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("workspace", failures[0])

    def test_a_crate_without_a_floor_fails(self):
        _, failures = floors.check(self.totals, config(0.0, **{"maidan-auth": 0.0}))
        self.assertEqual(len(failures), 1)
        self.assertIn("maidan-fsm", failures[0])
        self.assertIn("no floor", failures[0])

    def test_a_floor_without_a_crate_fails(self):
        _, failures = floors.check(
            self.totals,
            config(0.0, **{"maidan-auth": 0.0, "maidan-fsm": 0.0, "maidan-gone": 50.0}),
        )
        self.assertEqual(len(failures), 1)
        self.assertIn("maidan-gone", failures[0])


if __name__ == "__main__":
    unittest.main()
