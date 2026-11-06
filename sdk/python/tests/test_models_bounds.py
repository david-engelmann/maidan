"""Bounds on Model.from_dict / _convert — oversize and deep nesting fail closed."""

from __future__ import annotations

import unittest

from maidan.models import Workspace, _MAX_DEPTH, _MAX_LIST_LEN, _MAX_STR_LEN, from_list


def _workspace(**over):
    base = {
        "id": "ws_1",
        "name": "n",
        "created_at": "2026-01-01T00:00:00Z",
        "updated_at": "2026-01-01T00:00:00Z",
    }
    base.update(over)
    return base


class ModelBoundsTest(unittest.TestCase):
    def test_happy_path(self):
        ws = Workspace.from_dict(_workspace())
        self.assertEqual(ws.id, "ws_1")
        self.assertEqual(ws.extra, {})

    def test_oversize_known_string(self):
        with self.assertRaisesRegex(ValueError, "exceeds"):
            Workspace.from_dict(_workspace(name="x" * (_MAX_STR_LEN + 1)))

    def test_oversize_extra_string(self):
        with self.assertRaisesRegex(ValueError, "exceeds"):
            Workspace.from_dict(_workspace(unknown="y" * (_MAX_STR_LEN + 1)))

    def test_oversize_list_in_extra(self):
        with self.assertRaisesRegex(ValueError, "exceeds"):
            Workspace.from_dict(_workspace(bag=["a"] * (_MAX_LIST_LEN + 1)))

    def test_nesting_depth(self):
        bag: dict = {}
        cur = bag
        for _ in range(_MAX_DEPTH + 2):
            nxt: dict = {}
            cur["k"] = nxt
            cur = nxt
        with self.assertRaisesRegex(ValueError, "nesting exceeds"):
            Workspace.from_dict(_workspace(deep=bag))

    def test_from_list_cap(self):
        rows = [_workspace(id=f"ws_{i}") for i in range(3)]
        got = from_list(Workspace, rows)
        self.assertEqual(len(got), 3)
        with self.assertRaisesRegex(ValueError, "exceeds"):
            from_list(Workspace, [_workspace()] * (_MAX_LIST_LEN + 1))


if __name__ == "__main__":
    unittest.main()
