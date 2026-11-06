#!/usr/bin/env python3
"""Build a `pi.change.result/1` envelope from a local checkout.

The session claimant hands Maidan the commit it started from and a text diff;
Maidan commits it and opens the draft pull request. A diff a model copies by
hand rarely applies, so this takes it straight from git, checks it against
what Maidan's change flow accepts, and writes the request body for
`PUT /threads/{id}/result` (`{"result": <envelope>, "claim_lease_id": <lease>}`).

    python3 change_result.py --base-sha <sha> --branch feature/agent-<slug> \
        --claim-lease-id <lease> \
        --title "[ENG-123] ..." --summary-file body.md --linear-issue ENG-123 \
        --deliver-to-file deliver_to.json --out change-result.json

Exit status: 0 written; 2 a mistake to fix (an unclean tree, a bad branch, an
empty diff); 3 the change cannot go as a diff (a binary file, a symlink, a
submodule, a quoted path, or too large), so use the runbook's fallback.

Standard library only. No network: this never sees the token.
"""

import argparse
from pathlib import Path
import json
import re
import subprocess
import sys

SCHEMA = "maidan.waiter.result/1"
RESULT_KIND = "pi.change.result/1"
# Maidan writes only these branches (`maidan_types::is_change_branch`).
CHANGE_BRANCH = re.compile(r"^feature/agent-[a-z0-9][a-z0-9-]*$")
# `maidan_types::is_issue_key`.
ISSUE_KEY = re.compile(r"^[A-Z][A-Z0-9]*-[0-9]+$")
FULL_SHA = re.compile(r"^(?:[0-9a-f]{40}|[0-9a-f]{64})$")
# The server's default request limit is 2 MiB (`MAIDAN_MAX_BODY_BYTES`);
# leave room for the headers and a margin.
DEFAULT_MAX_BYTES = 2_000_000
# Modes the change flow refuses: a symlink and a submodule.
REFUSED_MODES = {"120000": "a symlink", "160000": "a submodule"}

FIX = 2
FALLBACK = 3
# A git call that outlives this (a lock, a credential prompt) would hold the
# claim with nobody watching; failing lets the session release it.
GIT_TIMEOUT_SECS = 120
class Stop(Exception):
    def __init__(self, code, message):
        super().__init__(message)
        self.code = code


REPO_NAME = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")
BASE_BRANCH = re.compile(r"^[A-Za-z0-9._/-]+$")
CLAIM_LEASE = re.compile(r"^[A-Za-z0-9._:~-]{8,128}$")
MIN_MAX_BYTES = 1024


def refuse_path(label, value):
    """Refuse path args that escape the working tree or look like redirects."""
    if value is None:
        return
    if "\x00" in value or "\n" in value or "\r" in value:
        raise Stop(FIX, f"{label} must not contain control characters")
    p = Path(value)
    if p.is_absolute():
        raise Stop(FIX, f"{label} must be a relative path, got {value!r}")
    if ".." in p.parts:
        raise Stop(FIX, f"{label} must not contain '..'")
    if value.startswith("~"):
        raise Stop(FIX, f"{label} must not expand a home directory")


def validate_inputs(args):
    if not CLAIM_LEASE.match(args.claim_lease_id):
        raise Stop(
            FIX,
            "claim-lease-id must be 8..128 characters of [A-Za-z0-9._:~-]",
        )
    if args.repo is not None and not REPO_NAME.match(args.repo):
        raise Stop(FIX, f"repo {args.repo!r} must look like owner/name")
    if args.base is not None:
        if args.base.lower() == "prod":
            raise Stop(FIX, "a change is never opened against prod")
        if not BASE_BRANCH.match(args.base) or ".." in args.base:
            raise Stop(FIX, f"base branch {args.base!r} is not a safe branch name")
    if args.max_bytes < MIN_MAX_BYTES or args.max_bytes > DEFAULT_MAX_BYTES:
        raise Stop(
            FIX,
            f"max-bytes must be {MIN_MAX_BYTES}..{DEFAULT_MAX_BYTES}, got {args.max_bytes}",
        )
    refuse_path("--out", args.out)
    refuse_path("--summary-file", args.summary_file)
    refuse_path("--deliver-to-file", args.deliver_to_file)
    # Absolute checkouts are allowed for --repo-dir (real agent paths), but
    # still refuse NUL and parent-segment escapes in the given string.
    if args.repo_dir is not None:
        if "\x00" in args.repo_dir:
            raise Stop(FIX, "--repo-dir must not contain NUL")
        if ".." in Path(args.repo_dir).parts:
            raise Stop(FIX, "--repo-dir must not contain '..'")



def git(repo_dir, *args):
    # Pin the settings a user's config could change: prefixes, colour,
    # external diff drivers and path quoting all alter the bytes Maidan parses.
    cmd = [
        "git",
        "-C",
        repo_dir,
        "-c",
        "core.quotePath=false",
        "-c",
        "diff.noprefix=false",
        "-c",
        "diff.mnemonicPrefix=false",
        *args,
    ]
    try:
        done = subprocess.run(
            cmd, capture_output=True, text=True, encoding="utf-8", timeout=GIT_TIMEOUT_SECS
        )
    except subprocess.TimeoutExpired:
        raise Stop(FIX, f"git {' '.join(args)} did not finish in {GIT_TIMEOUT_SECS} s")
    except UnicodeDecodeError:
        # The envelope is JSON text; a diff that is not UTF-8 cannot ride in it.
        raise Stop(FALLBACK, f"git {' '.join(args)} printed bytes that are not UTF-8")
    if done.returncode != 0:
        raise Stop(FIX, f"git {' '.join(args)}: {done.stderr.strip()}")
    return done.stdout


def resolve(repo_dir, rev):
    return git(repo_dir, "rev-parse", "--verify", f"{rev}^{{commit}}").strip()


def check_tree_clean(repo_dir):
    if git(repo_dir, "status", "--porcelain", "--untracked-files=no").strip():
        raise Stop(FIX, "uncommitted changes: commit them first, the diff is base..HEAD")


def check_shape(repo_dir, base, head):
    """Refuse what the change flow would refuse, before Maidan has to."""
    for line in git(repo_dir, "diff", "--numstat", "--find-renames", base, head).splitlines():
        fields = line.split("\t", 2)
        if len(fields) < 3:
            raise Stop(FIX, f"git diff --numstat printed an unexpected line: {line!r}")
        added, removed, path = fields
        if added == "-" and removed == "-":
            raise Stop(FALLBACK, f"{path} is a binary file; the change flow commits text only")
    for line in git(repo_dir, "diff", "--raw", "--find-renames", base, head).splitlines():
        meta, _, path = line.partition("\t")
        fields = meta.lstrip(":").split()
        for mode in fields[:2]:
            if mode in REFUSED_MODES:
                raise Stop(FALLBACK, f"{path} is {REFUSED_MODES[mode]}, which the change flow refuses")
        if fields and fields[-1].startswith(("C", "T")):
            raise Stop(FALLBACK, f"{path} is a copy or a type change, which the change flow refuses")


def text_diff(repo_dir, base, head):
    diff = git(
        repo_dir,
        "diff",
        "--no-color",
        "--no-ext-diff",
        "--no-textconv",
        "--find-renames",
        "--src-prefix=a/",
        "--dst-prefix=b/",
        base,
        head,
    )
    if not diff.strip():
        raise Stop(FIX, "the diff is empty: send status no_change instead")
    if any(line.startswith('diff --git "') for line in diff.splitlines()):
        raise Stop(FALLBACK, "a path needs quoting, which the change flow refuses")
    return diff


def deliver_to(args):
    targets = []
    if args.deliver_to_file:
        with open(args.deliver_to_file, encoding="utf-8") as fh:
            targets = json.load(fh)
        if not isinstance(targets, list) or not all(isinstance(t, dict) for t in targets):
            raise Stop(FIX, "--deliver-to-file must hold a JSON array of target objects")
    branch_targets = [t for t in targets if t.get("surface") == "github_branch"]
    for target in branch_targets:
        if target.get("branch") != args.branch:
            raise Stop(
                FIX,
                f"deliver_to names branch {target.get('branch')!r}; the result must be for that branch, not {args.branch!r}",
            )
    if not branch_targets:
        if not (args.repo and args.base):
            raise Stop(FIX, "deliver_to has no github_branch target: pass --repo and --base to add one")
        targets.append({"surface": "github_branch", "repo": args.repo, "branch": args.branch, "base": args.base})
    for target in targets:
        if target.get("surface") == "github_branch" and str(target.get("base", "")).lower() == "prod":
            raise Stop(FIX, "a change is never opened against prod")
    return targets


def build(args):
    validate_inputs(args)
    if not CHANGE_BRANCH.match(args.branch):
        raise Stop(FIX, f"branch {args.branch!r} must match feature/agent-[a-z0-9][a-z0-9-]*")
    if args.linear_issue and not ISSUE_KEY.match(args.linear_issue):
        raise Stop(FIX, f"{args.linear_issue!r} is not an issue key such as ENG-123")
    check_tree_clean(args.repo_dir)
    base = resolve(args.repo_dir, args.base_sha)
    if not FULL_SHA.match(base):
        raise Stop(FIX, f"base {args.base_sha!r} did not resolve to a full SHA")
    head = resolve(args.repo_dir, args.head)
    check_shape(args.repo_dir, base, head)
    envelope = {
        "schema": SCHEMA,
        "result_kind": RESULT_KIND,
        "status": "changed",
        "base_sha": base,
        "branch": args.branch,
        "diff": text_diff(args.repo_dir, base, head),
        "deliver_to": deliver_to(args),
    }
    if args.title:
        envelope["title"] = args.title
    if args.summary_file:
        with open(args.summary_file, encoding="utf-8") as fh:
            envelope["summary"] = fh.read()
    if args.linear_issue:
        envelope["linear_issue"] = args.linear_issue
    # The claim fence: Maidan stores the result only while this lease is the
    # thread's live claim, so a seat whose claim lapsed and was taken over
    # cannot land its work over the new holder's.
    body = json.dumps({"result": envelope, "claim_lease_id": args.claim_lease_id}, ensure_ascii=False)
    size = len(body.encode("utf-8"))
    if size > args.max_bytes:
        raise Stop(FALLBACK, f"the result is {size} bytes, over the {args.max_bytes}-byte limit")
    return body, size


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("--base-sha", required=True, help="the commit the work started from, read before any edit")
    parser.add_argument("--branch", required=True, help="feature/agent-<slug>")
    parser.add_argument("--claim-lease-id", required=True,
                        help="the claim's lease, the claim response's claim_lease_id")
    parser.add_argument("--head", default="HEAD", help="the last commit of the work (default HEAD)")
    parser.add_argument("--title", help="the pull request title")
    parser.add_argument("--summary-file", help="a file holding the pull request body")
    parser.add_argument("--linear-issue", help="the tracker issue key, such as ENG-123")
    parser.add_argument("--deliver-to-file", help="a JSON file holding the opening message's deliver_to list")
    parser.add_argument("--repo", help="owner/name, for a github_branch target deliver_to lacks")
    parser.add_argument("--base", help="the pull request's base branch, for that target")
    parser.add_argument("--repo-dir", default=".", help="the checkout (default: the current directory)")
    parser.add_argument("--max-bytes", type=int, default=DEFAULT_MAX_BYTES)
    parser.add_argument("--out", help="where to write the request body (default: stdout)")
    args = parser.parse_args(argv)
    try:
        body, size = build(args)
    except Stop as stop:
        label = "use the fallback" if stop.code == FALLBACK else "fix"
        print(f"change_result: {stop} ({label})", file=sys.stderr)
        return stop.code
    except (OSError, json.JSONDecodeError) as err:
        print(f"change_result: {err} (fix)", file=sys.stderr)
        return FIX
    if args.out:
        with open(args.out, "w", encoding="utf-8") as fh:
            fh.write(body)
    else:
        sys.stdout.write(body)
    print(f"change_result: {size} bytes for {args.branch}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
