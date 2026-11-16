#!/usr/bin/env python3
"""Example prompts per listing, as a runnable script (Next 14).

    scripts/listing-prompts.py --check
        Every prompt's expected tools exist in the MCP catalog
        (listings/docker/tools.json), every listing names known prompts, and
        each listing's prompts expect only tools the endpoint it lists serves
        ("surfaces": /mcp/streamable is the whole catalog, a profile endpoint
        its list in crates/maidan-mcp/src/profiles.rs). Where the listing's
        package names its endpoint, the package and "surfaces" must agree.
    scripts/listing-prompts.py --listing cursor
        Print that listing's prompts and the tools each should pick.
    scripts/listing-prompts.py --listing claude --run 'claude -p {prompt}'
        Run each prompt through a platform's own model, already connected to
        Maidan, and print the validation record's table. {prompt} is replaced
        with the shell-quoted prompt; without it the prompt goes on stdin. The
        tools picked are the catalog tool names the command's output mentions,
        so point the command at a transcript or verbose output that names the
        tool calls (for example `--output-format stream-json`). A prompt passes
        when every expected tool was picked; a negative prompt passes when no
        Maidan tool was.

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
PROFILES = ROOT / "crates" / "maidan-mcp" / "src" / "profiles.rs"
FULL = "/mcp/streamable"
# Packages that name the endpoint they connect to: the listing, the file, and
# how to read the endpoint's URL out of it.
MANIFESTS = {
    "cursor": (".cursor-plugin/plugin.json", lambda m: m["mcpServers"]["maidan"]["url"]),
    "gemini-cli": ("gemini-extension.json", lambda m: next(iter(m["mcpServers"].values()))["httpUrl"]),
    "registry": ("listings/registry/server.json", lambda m: m["remotes"][0]["url"]),
}


def load() -> tuple[dict, set[str]]:
    data = json.loads(PROMPTS.read_text())
    tools = {t["name"] for t in json.loads(CATALOG.read_text())["tools"]}
    return data, tools


def profile_tools() -> dict[str, set[str]]:
    """Each profile endpoint's tools, read from the Rust lists the server serves."""
    src = PROFILES.read_text()
    out = {}
    for name in re.findall(r"pub const ([A-Z]+)_TOOLS: &\[&str\]", src):
        body = re.search(name + r"_TOOLS: &\[&str\] = &\[(.*?)\];", src, re.S)
        out[f"/mcp/{name.lower()}"] = set(re.findall(r'"([a-z_]+)"', body.group(1)))
    return out


def check(data: dict, tools: set[str]) -> list[str]:
    problems = []
    for key, p in data["prompts"].items():
        if not p.get("prompt", "").strip():
            problems.append(f"prompt {key}: empty prompt")
        if p.get("negative"):
            if p.get("expect"):
                problems.append(f"prompt {key}: a negative prompt expects no tool")
        elif not p.get("expect"):
            problems.append(f"prompt {key}: no expected tools")
        for t in p.get("expect", []):
            if t not in tools:
                problems.append(f"prompt {key}: {t} is not in the MCP catalog")
    surfaces = {FULL: tools, **profile_tools()}
    for listing, keys in data["listings"].items():
        if not keys:
            problems.append(f"listing {listing}: no prompts")
        surface = data.get("surfaces", {}).get(listing)
        if surface not in surfaces:
            problems.append(f"listing {listing}: surface {surface!r} is not one of {', '.join(sorted(surfaces))}")
        for k in keys:
            if k not in data["prompts"]:
                problems.append(f"listing {listing}: unknown prompt {k}")
                continue
            for t in data["prompts"][k].get("expect", []):
                if surface in surfaces and t not in surfaces[surface]:
                    problems.append(f"listing {listing}: prompt {k} expects {t}, which {surface} does not serve")
    for listing, (path, url) in MANIFESTS.items():
        try:
            endpoint = url(json.loads((ROOT / path).read_text()))
        except (OSError, KeyError, StopIteration, IndexError, json.JSONDecodeError) as err:
            problems.append(f"listing {listing}: cannot read its endpoint from {path} ({err!r})")
            continue
        if not endpoint.endswith(data.get("surfaces", {}).get(listing) or "\0"):
            problems.append(f"listing {listing}: {path} connects to {endpoint}, not its surface {data.get('surfaces', {}).get(listing)}")
    return problems


def picked(output: str, tools: set[str]) -> list[str]:
    # Claude Code names MCP tools mcp__<server>__<tool>; keep the part after
    # the last "__" so namespaced calls match the catalog name exactly.
    words = {w.rsplit("__", 1)[-1] for w in re.findall(r"[a-z][a-z0-9_]+", output)}
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
        # A run that timed out or failed picked nothing, which a negative
        # prompt would otherwise count as a pass: judge only completed runs.
        try:
            out = subprocess.run(argv, shell=True, input=stdin, capture_output=True, text=True, timeout=timeout)
            text = out.stdout + out.stderr
            broken = f"exit {out.returncode}" if out.returncode else ""
        except subprocess.TimeoutExpired:
            text, broken = "", f"timed out after {timeout} s"
        got = picked(text, tools)
        ok = not broken and (not got if p.get("negative") else all(t in got for t in p["expect"]))
        failed += 0 if ok else 1
        verdict = "yes" if ok else f"no ({broken})" if broken else "no"
        print(f"| {cell(p['prompt'])} | {', '.join(p['expect']) or '(none: negative)'} | {', '.join(got) or '(none)'} | {verdict} |")
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
        print(f"{p['prompt']}\n  expect: {', '.join(p['expect']) or 'no Maidan tool (negative)'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
