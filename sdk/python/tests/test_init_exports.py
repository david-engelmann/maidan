"""Public package surface: semver version and fail-closed unknown attrs."""

from __future__ import annotations

import importlib
import re
import unittest


class InitExportsTest(unittest.TestCase):
    def test_version_is_semver_shaped(self) -> None:
        import maidan

        self.assertRegex(maidan.__version__, r"^\d+\.\d+\.\d+[a-z0-9.-]*$")

    def test_unknown_attribute_raises(self) -> None:
        import maidan

        with self.assertRaises(AttributeError) as ctx:
            _ = maidan.definitely_not_an_export_zz  # type: ignore[attr-defined]
        self.assertIn("definitely_not_an_export_zz", str(ctx.exception))

    def test_all_entries_are_exportable(self) -> None:
        import maidan

        for name in maidan.__all__:
            self.assertTrue(hasattr(maidan, name), name)


if __name__ == "__main__":
    unittest.main()
