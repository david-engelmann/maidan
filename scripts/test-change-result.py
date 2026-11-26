#!/usr/bin/env python3
"""Tests for examples/claude-code/maidan-claimant/change_result.py.
Run: python3 scripts/test-change-result.py

Each test builds a throwaway git repository and runs the helper against it, so
the diff it prints is the one a session would send. Standard library and git
only."""

import contextlib
import importlib.util
import io
import json
import os
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SPEC = importlib.util.spec_from_file_location(
    "change_result", ROOT / "examples/claude-code/maidan-claimant/change_result.py"
)
change_result = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(change_result)

BRANCH = "feature/agent-test-change"
LEASE = "00000000-0000-0000-0000-000000000001"
BASE_TARGET = {"surface": "github_branch", "repo": "owner/name", "branch": BRANCH, "base": "main"}


def git(repo, *args):
    env = dict(
        os.environ,
        GIT_AUTHOR_NAME="t",
        GIT_AUTHOR_EMAIL="t@example.com",
        GIT_COMMITTER_NAME="t",
        GIT_COMMITTER_EMAIL="t@example.com",
        GIT_CONFIG_GLOBAL=os.devnull,
        GIT_CONFIG_SYSTEM=os.devnull,
    )
    done = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, env=env)
    if done.returncode != 0:
        raise AssertionError(done.stderr.decode())
    return done.stdout.decode().strip()


def write(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(data)


class ChangeResultTest(unittest.TestCase):
    def setUp(self):
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        self.repo = Path(tmp.name) / "repo"
        self.repo.mkdir()
        git(self.repo, "init", "-q")
        git(self.repo, "config", "core.autocrlf", "false")
        write(self.repo / "crlf.txt", b"one\r\ntwo\r\nthree\r\n")
        write(self.repo / "sub/inside.txt", b"inside\n")
        write(self.repo / "outside.txt", b"outside\n")
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-q", "-m", "base")
        self.base = git(self.repo, "rev-parse", "HEAD")
        self.deliver_to = Path(tmp.name) / "deliver_to.json"
        self.deliver_to.write_text(json.dumps([BASE_TARGET]))
        self.out = Path(tmp.name) / "out.json"

    def commit(self):
        git(self.repo, "add", ".")
        git(self.repo, "commit", "-q", "-m", "work")

    def run_helper(self, repo_dir=None):
        argv = [
            "--base-sha", self.base,
            "--branch", BRANCH,
            "--claim-lease-id", LEASE,
            "--deliver-to-file", str(self.deliver_to),
            "--repo-dir", str(repo_dir or self.repo),
            "--out", str(self.out),
        ]
        with contextlib.redirect_stderr(io.StringIO()) as err:
            code = change_result.main(argv)
        return code, err.getvalue()

    def diff(self):
        return json.loads(self.out.read_text(encoding="utf-8"))["result"]["diff"]

    def test_a_crlf_file_keeps_its_carriage_returns_in_the_diff(self):
        write(self.repo / "crlf.txt", b"one\r\nTWO\r\nthree\r\n")
        write(self.repo / "new-crlf.txt", b"a\r\nb\r\n")
        self.commit()
        code, err = self.run_helper()
        self.assertEqual(code, 0, err)
        diff = self.diff()
        self.assertIn(" one\r\n", diff)
        self.assertIn("-two\r\n", diff)
        self.assertIn("+TWO\r\n", diff)
        self.assertIn("+a\r\n+b\r\n", diff)

    def test_the_diff_keeps_repository_root_paths_when_diff_relative_is_set(self):
        git(self.repo, "config", "diff.relative", "true")
        write(self.repo / "sub/new.txt", b"new\n")
        write(self.repo / "outside.txt", b"changed\n")
        self.commit()
        code, err = self.run_helper(repo_dir=self.repo / "sub")
        self.assertEqual(code, 0, err)
        diff = self.diff()
        self.assertIn("diff --git a/sub/new.txt b/sub/new.txt", diff)
        self.assertIn("diff --git a/outside.txt b/outside.txt", diff)

    def test_a_diff_that_is_not_utf8_goes_to_the_fallback(self):
        write(self.repo / "latin1.txt", "caf\xe9\n".encode("latin-1"))
        self.commit()
        code, err = self.run_helper()
        self.assertEqual(code, change_result.FALLBACK, err)
        self.assertIn("not UTF-8", err)

    def test_a_binary_file_goes_to_the_fallback(self):
        write(self.repo / "blob.bin", b"\x00\x01\x02")
        self.commit()
        code, err = self.run_helper()
        self.assertEqual(code, change_result.FALLBACK, err)

    def test_an_empty_diff_is_a_mistake_to_fix(self):
        git(self.repo, "commit", "-q", "--allow-empty", "-m", "nothing")
        code, err = self.run_helper()
        self.assertEqual(code, change_result.FIX, err)


if __name__ == "__main__":
    unittest.main()
