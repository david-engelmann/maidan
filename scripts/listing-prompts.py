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
    words = set(re.findall(r"[a-z][a-z0-9_]+", output))
    return sorted(words & tools)


def cell(s: str) -> str:
    return s.replace("|", "\\|").replace("\n", " ")


def run(data: dict, tools: set[str], listing: str, cmd: str, timeout: int) -> int:
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
    if args.listing not in data["listings"]:
        ap.error(f"unknown listing {args.listing}; known: {', '.join(sorted(data['listings']))}")
    if args.run:
        return run(data, tools, args.listing, args.run, args.timeout)
    for key in data["listings"][args.listing]:
        p = data["prompts"][key]
        print(f"{p['prompt']}\n  expect: {', '.join(p['expect'])}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
