#!/usr/bin/env python3
"""Example prompts per listing, as a runnable script (Next 14).

    scripts/listing-prompts.py --check
        Every prompt's expected tools exist in the MCP catalog
        (listings/docker/tools.json), and every listing names known prompts.
    scripts/listing-prompts.py --listing cursor
        Print that listing's prompts and the tools each should pick.
    scripts/listing-prompts.py --listing claude --run 'claude -p {prompt}'
        Run each prompt through a platform's own model, already connected to
        Maidan, and print the validation record's table. {prompt} is replaced
        with the shell-quoted prompt; without it the prompt goes on stdin. The
        tools picked are the catalog tool names the command's output mentions,
        so point the command at a transcript or verbose output that names the
        tool calls (for example `--output-format stream-json`). A prompt passes
        when every expected tool was picked.

Exit status: --check and --run exit 1 on any failure.
"""
from __future__ import annotations

import argparse
import json
import re
import shlex
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
PROMPTS = ROOT / "listings" / "prompts.json"
CATALOG = ROOT / "listings" / "docker" / "tools.json"

_LISTING_RE = re.compile(r"^[a-z][a-z0-9-]{0,63}$")
_MAX_TIMEOUT = 600
_MIN_TIMEOUT = 1
_MAX_RUN = 4000
_RUN_SAFE = re.compile(r"^[\w ./{}:@%+=,_-]+$")


def validate_listing(listing: str, data: dict) -> str:
    """Refuse unknown or oddly shaped listing ids before any prompt run."""
    if not isinstance(listing, str) or not _LISTING_RE.match(listing):
        raise ValueError("listing id is invalid")
    if listing not in data.get("listings", {}):
        raise ValueError(f"unknown listing: {listing}")
    return listing


def validate_timeout(timeout: int) -> int:
    if isinstance(timeout, bool) or not isinstance(timeout, int):
        raise ValueError("timeout must be an int")
    if timeout < _MIN_TIMEOUT or timeout > _MAX_TIMEOUT:
        raise ValueError(f"timeout must be {_MIN_TIMEOUT}..{_MAX_TIMEOUT}")
    return timeout


def validate_run_cmd(cmd: str) -> str:
    """Refuse empty/oversized --run strings and shell expansion metacharacters."""
    if not isinstance(cmd, str) or not cmd.strip():
        raise ValueError("run command is empty")
    if len(cmd) > _MAX_RUN:
        raise ValueError("run command is too long")
    forbidden = ("`", "$" + "(", "$" + "{", chr(10), chr(13), ";", "|", "&", ">", "<")
    if any(token in cmd for token in forbidden):
        raise ValueError("run command has unsafe shell metacharacters")
    if not _RUN_SAFE.match(cmd.replace("{prompt}", "PROMPT")):
        raise ValueError("run command has invalid characters")
    return cmd



def load() -> tuple[dict, set[str]]:
    data = json.loads(PROMPTS.read_text())
    tools = {t["name"] for t in json.loads(CATALOG.read_text())["tools"]}
    return data, tools


def check(data: dict, tools: set[str]) -> list[str]:
    problems = []
    for key, p in data["prompts"].items():
        if not p.get("prompt", "").strip():
            problems.append(f"prompt {key}: empty prompt")
        if not p.get("expect"):
            problems.append(f"prompt {key}: no expected tools")
        for t in p.get("expect", []):
            if t not in tools:
                problems.append(f"prompt {key}: {t} is not in the MCP catalog")
    for listing, keys in data["listings"].items():
        if not keys:
            problems.append(f"listing {listing}: no prompts")
        for k in keys:
            if k not in data["prompts"]:
                problems.append(f"listing {listing}: unknown prompt {k}")
    return problems


def picked(output: str, tools: set[str]) -> list[str]:
    # Claude Code names MCP tools mcp__<server>__<tool>; keep the part after
    # the last "__" so namespaced calls match the catalog name exactly.
    words = {w.rsplit("__", 1)[-1] for w in re.findall(r"[a-z][a-z0-9_]+", output)}
    return sorted(words & tools)


def cell(s: str) -> str:
    return s.replace("|", "\\|").replace("\n", " ")


def run(data: dict, tools: set[str], listing: str, cmd: str, timeout: int) -> int:
    listing = validate_listing(listing, data)
    cmd = validate_run_cmd(cmd)
    timeout = validate_timeout(timeout)
    print("| Prompt | Tools expected | Tools picked | Pass |")
    print("|---|---|---|---|")
    failed = 0
    for key in data["listings"][listing]:
        p = data["prompts"][key]
        if "{prompt}" in cmd:
            argv, stdin = cmd.replace("{prompt}", shlex.quote(p["prompt"])), None
        else:
            argv, stdin = cmd, p["prompt"]
        try:
            out = subprocess.run(argv, shell=True, input=stdin, capture_output=True, text=True, timeout=timeout)
            text = out.stdout + out.stderr
        except subprocess.TimeoutExpired:
            text = ""
        got = picked(text, tools)
        ok = all(t in got for t in p["expect"])
        failed += 0 if ok else 1
        print(f"| {cell(p['prompt'])} | {', '.join(p['expect'])} | {', '.join(got) or '(none)'} | {'yes' if ok else 'no'} |")
    return 1 if failed else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--check", action="store_true")
    ap.add_argument("--listing")
    ap.add_argument("--run", metavar="CMD")
    ap.add_argument("--timeout", type=int, default=300, help="seconds per prompt (default 300)")
    args = ap.parse_args()
    data, tools = load()
    if args.check:
        problems = check(data, tools)
        for p in problems:
            print(p, file=sys.stderr)
        if not problems:
            print(f"ok: {len(data['prompts'])} prompts across {len(data['listings'])} listings")
        return 1 if problems else 0
    if not args.listing:
        ap.error("--listing or --check is required")
    try:
        listing = validate_listing(args.listing, data)
        if args.run:
            validate_run_cmd(args.run)
        validate_timeout(args.timeout)
    except ValueError as exc:
        ap.error(str(exc))
    if args.run:
        return run(data, tools, listing, args.run, args.timeout)
    args.listing = listing
    for key in data["listings"][args.listing]:
        p = data["prompts"][key]
        print(f"{p['prompt']}\n  expect: {', '.join(p['expect'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
