#!/usr/bin/env python3
"""Unit checks for CI/deploy gate hardening (no cargo, docker, or network)."""

from __future__ import annotations

import os
import re
import stat
import subprocess
import tempfile
import textwrap
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


class DeployContractScript(unittest.TestCase):
    def test_no_rg_dependency(self) -> None:
        src = (ROOT / "scripts/check-deploy-contract.sh").read_text()
        self.assertNotRegex(src, r"(?m)^\s*rg\s")
        self.assertIn("refusing to pass", src)
        self.assertIn("grep -RInE", src)

    def test_loose_refs_fail_closed(self) -> None:
        src = (ROOT / "scripts/check-deploy-contract.sh").read_text()
        self.assertIn("failed scanning cgr.dev/chainguard", src)
        self.assertNotIn("|| true)", src)


class DeployPinsScript(unittest.TestCase):
    def test_version_gt_rejects_metacharacters(self) -> None:
        script = textwrap.dedent(
            r"""
            set -euo pipefail
            source scripts/check-deploy-pins.sh
            """
        )
        # Cannot source full script (it runs). Extract version_gt via bash -c copy.
        src = (ROOT / "scripts/check-deploy-pins.sh").read_text()
        m = re.search(r"version_gt\(\) \{.*?\n\}", src, re.S)
        self.assertIsNotNone(m)
        fn = m.group(0)
        probe = fn + '\nversion_gt "1.2.3" "1.2.2"; echo gt_ok\n'
        probe += 'version_gt "1.2.3;rm" "1.0.0" && echo bad || echo reject_meta\n'
        probe += 'version_gt "1.2.3" "nope" && echo bad2 || echo reject_right\n'
        out = subprocess.check_output(["bash", "-c", probe], text=True)
        self.assertIn("gt_ok", out)
        self.assertIn("reject_meta", out)
        self.assertIn("reject_right", out)

    def test_pin_grep_fail_closed(self) -> None:
        src = (ROOT / "scripts/check-deploy-pins.sh").read_text()
        self.assertIn('grep failed ($grc)', src)
        self.assertNotIn('grep -noE "$pattern" "$file" || true', src)


class OsvScanScript(unittest.TestCase):
    def test_hash_before_install(self) -> None:
        src = (ROOT / "scripts/osv-scan.sh").read_text()
        part = src.index("verify_scanner")
        mv = src.index('mv "$scanner.part" "$scanner"')
        chmod = src.index('chmod +x "$scanner"')
        self.assertLess(part, mv)
        self.assertLess(mv, chmod)
        self.assertIn("does not match the pinned sha256", src)

    def test_override_is_hash_checked(self) -> None:
        src = (ROOT / "scripts/osv-scan.sh").read_text()
        # Existing file path (OSV_SCANNER) must call verify_scanner
        self.assertIn('elif ! verify_scanner "$scanner"', src)


class AlertRulesScript(unittest.TestCase):
    def test_missing_rules_fail_without_promtool(self) -> None:
        src = (ROOT / "scripts/check-alert-rules.sh").read_text()
        # File existence checks appear before the promtool soft-skip.
        miss = src.index('missing $RULES')
        skip = src.index("promtool not found")
        self.assertLess(miss, skip)
        self.assertIn("has no groups", src)

    def test_missing_file_exits_nonzero(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            tmp_p = Path(tmp)
            # Minimal fake repo with the script but without the rules file.
            (tmp_p / "scripts").mkdir()
            script = (ROOT / "scripts/check-alert-rules.sh").read_text()
            (tmp_p / "scripts/check-alert-rules.sh").write_text(script)
            (tmp_p / "docs/alerts").mkdir(parents=True)
            env = {**os.environ, "PATH": "/usr/bin:/bin"}
            # Ensure promtool is "missing" by emptying PATH of it; still must fail on missing RULES.
            r = subprocess.run(
                ["bash", str(tmp_p / "scripts/check-alert-rules.sh")],
                cwd=tmp_p,
                env=env,
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(r.returncode, 0)
            self.assertIn("missing", r.stderr.lower())


class ReviewerTokenScript(unittest.TestCase):
    def test_url_and_wait_guards_present(self) -> None:
        src = (ROOT / "scripts/check-reviewer-token-expiry.sh").read_text()
        self.assertIn("must be an http(s) URL", src)
        self.assertIn("no userinfo", src)
        self.assertIn("between 1 and 365", src)

    def test_rejects_bad_url_before_network(self) -> None:
        env = {
            **os.environ,
            "MAIDAN_URL": "file:///etc/passwd",
            "MAIDAN_REVIEWER_TOKEN": "tok",
            "MAIDAN_ADMIN_TOKEN": "tok",
        }
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-reviewer-token-expiry.sh")],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("http(s)", r.stderr)

    def test_rejects_userinfo_url(self) -> None:
        env = {
            **os.environ,
            "MAIDAN_URL": "https://user:pass@example.com",
            "MAIDAN_REVIEWER_TOKEN": "tok",
            "MAIDAN_ADMIN_TOKEN": "tok",
        }
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-reviewer-token-expiry.sh")],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 2)

    def test_rejects_out_of_range_wait(self) -> None:
        env = {
            **os.environ,
            "MAIDAN_URL": "http://127.0.0.1:8080",
            "MAIDAN_REVIEWER_TOKEN": "tok",
            "MAIDAN_ADMIN_TOKEN": "tok",
            "MAIDAN_REVIEW_WAIT_DAYS": "0",
        }
        r = subprocess.run(
            ["bash", str(ROOT / "scripts/check-reviewer-token-expiry.sh")],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("1 and 365", r.stderr)


if __name__ == "__main__":
    unittest.main()
