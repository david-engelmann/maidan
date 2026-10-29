#!/usr/bin/env python3
"""Generate Maidan's HOL Guard MCP server contributions from the tool tiers.

HOL Guard (github.com/hashgraph-online/hol-guard) reviews an agent's tool
calls on the person's machine. A contribution tells it, per tool, whether a
call to Maidan inherits the person's own Guard policy, needs their review, or
is blocked. The contributions are generated, never edited: the inputs are

- listings/hol-guard/policy.json, a risk tier for every MCP tool;
- crates/maidan-mcp/tests/fixtures/tool-annotations.json, the reviewed hints;
- crates/maidan-mcp/src/profiles.rs, the fixed tool lists of /mcp/<profile>.

The policy is checked before anything is written: every tool has one tier,
the read tier is exactly the read-only tools, and a write that reaches outside
Maidan is in a reviewed tier unless the policy says why not.

    python3 scripts/gen-hol-guard.py           # write listings/hol-guard/mcp.*.json
    python3 scripts/gen-hol-guard.py --check   # fail when they are stale (CI)
    python3 scripts/gen-hol-guard.py --local   # the same policy as Guard custom-extension choices

A contribution only matches the hosted HTTPS endpoints or the maidan binary,
so a Maidan on this machine over http is set up by hand in Guard as a custom
extension; --local prints which tools to set to Allow, Recommended and Block.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
OUT = ROOT / "listings" / "hol-guard"
POLICY = OUT / "policy.json"
ANNOTATIONS = ROOT / "crates" / "maidan-mcp" / "tests" / "fixtures" / "tool-annotations.json"
PROFILES = ROOT / "crates" / "maidan-mcp" / "src" / "profiles.rs"

HOSTED = "https://maidan.world"
REPO = "https://github.com/david-engelmann/maidan"
REVIEWED = {"review", "block"}
STATES = {"inherit", "review", "block"}  # Guard refuses allow from a remote or direct-command server.
MAX_NAMED = 79  # Guard's schema caps tools at 80, and `other` takes one.

PUBLISHER = {
    "id": "io.github.david-engelmann",
    "displayName": "Maidan",
    "url": REPO,
}
COMMON_ALTERNATIVES = [
    "Give each agent its own seat token holding maidan.agent.worker, never an admin token.",
    "Name secrets as secret:// references in payloads so their values never enter a model's context.",
]


def fail(message: str) -> None:
    print(f"FAIL: {message}")
    sys.exit(1)


def load_policy() -> tuple[dict, dict[str, str]]:
    policy = json.loads(POLICY.read_text())
    hints = json.loads(ANNOTATIONS.read_text())["tools"]
    tiers = policy["tiers"]
    tools = policy["tools"]
    errors = []
    for tier, row in tiers.items():
        if row.get("guard") not in STATES:
            errors.append(f"tier {tier} maps to {row.get('guard')!r}, not one of {sorted(STATES)}")
    for name in sorted(set(hints) - set(tools)):
        errors.append(f"{name} has no tier in {POLICY.relative_to(ROOT)}")
    for name in sorted(set(tools) - set(hints)):
        errors.append(f"{POLICY.relative_to(ROOT)} tiers {name}, which is not a tool")
    exceptions = policy.get("open_world_exceptions", {})
    for name, tier in sorted(tools.items()):
        if name not in hints:
            continue
        if tier not in tiers:
            errors.append(f"{name} has the unknown tier {tier!r}")
            continue
        read_only = hints[name]["readOnlyHint"]["value"]
        if (tier == "read") != read_only:
            errors.append(f"{name}: the read tier must be exactly the read-only tools (readOnlyHint {read_only})")
        reaches_out = hints[name]["openWorldHint"]["value"] and not read_only
        if reaches_out and tiers[tier]["guard"] not in REVIEWED and name not in exceptions:
            errors.append(f"{name} writes outside Maidan, so its tier must be reviewed or the policy must say why not")
    for name in sorted(set(exceptions) - set(tools)):
        errors.append(f"open_world_exceptions names {name}, which is not a tool")
    if errors:
        fail("\nFAIL: ".join(errors))
    return policy, {name: tiers[tier]["guard"] for name, tier in tools.items()}


def load_profiles() -> dict[str, list[str]]:
    source = PROFILES.read_text()
    profiles = {}
    for name, body in re.findall(r"pub const ([A-Z]+)_TOOLS: &\[&str\] = &\[(.*?)\];", source, re.S):
        profile = name.lower()
        if f'"/mcp/{profile}"' not in source:
            fail(f"{PROFILES.relative_to(ROOT)}: {name}_TOOLS has no /mcp/{profile} endpoint")
        profiles[profile] = re.findall(r'"([a-z0-9_]+)"', body)
    if not profiles:
        fail(f"no profile tool lists found in {PROFILES.relative_to(ROOT)}")
    return profiles


def contribution(*, slug: str, name: str, description: str, icon: str, launch: dict,
                 risk_classes: list[str], tools: list[dict], alternatives: list[str],
                 version: str) -> dict:
    if len(tools) > MAX_NAMED + 1:
        fail(f"mcp.{slug}: {len(tools)} tool rows, Guard takes at most {MAX_NAMED + 1}")
    return {
        "schemaVersion": "guard.mcp-server-contribution.v1",
        "id": f"mcp.{slug}",
        "version": version,
        "name": name,
        "description": description,
        "trustClass": "external",
        "activation": "opt-in",
        "publisher": PUBLISHER,
        "icon": {"kind": "react-icon", "name": icon, "background": "#1F2937"},
        "homepage": REPO,
        "license": "MIT",
        "launch": launch,
        "riskClasses": risk_classes,
        "tools": tools,
        "referenceUrls": [
            f"{REPO}/blob/main/listings/hol-guard/README.md",
            f"{REPO}/blob/main/listings/hol-guard/policy.json",
            f"{REPO}/blob/main/crates/maidan-mcp/tests/fixtures/tool-annotations.json",
            f"{REPO}/blob/main/docs/Integration.md",
        ],
        "saferAlternatives": alternatives,
    }


def full_surface_tools(states: dict[str, str]) -> list[dict]:
    """Every reviewed or blocked tool by name; the rest inherit the person's policy."""
    named = sorted(name for name, state in states.items() if state in REVIEWED)
    if len(named) > MAX_NAMED:
        fail(f"{len(named)} tools are reviewed or blocked; Guard names at most {MAX_NAMED}")
    return [{"name": name, "state": states[name]} for name in named] + [{"name": "other", "state": "inherit"}]


def profile_tools(names: list[str], states: dict[str, str]) -> list[dict]:
    """A profile names every tool it serves; anything else it might add is reviewed."""
    unknown = sorted(set(names) - set(states))
    if unknown:
        fail(f"profile lists tools with no tier: {', '.join(unknown)}")
    return [{"name": name, "state": states[name]} for name in sorted(names)] + [{"name": "other", "state": "review"}]


def risk_classes(policy: dict, names: list[str]) -> list[str]:
    tiers = {policy["tools"][name] for name in names}
    return sorted(f"maidan-{tier}" for tier in tiers if policy["tiers"][tier]["guard"] in REVIEWED)


def build() -> dict[str, dict]:
    policy, states = load_policy()
    version = policy["version"]
    everything = sorted(states)
    full_alternatives = [
        "Connect agents that only work or review tasks to the /mcp/worker or /mcp/reviewer profile endpoints, which list only that loop's tools.",
        *COMMON_ALTERNATIVES,
    ]
    files = {
        "maidan": contribution(
            slug="maidan",
            name="Maidan",
            description=(
                "Maidan, the workspace where people and AI agents share threads, tasks, reviews and approvals, "
                "at its full MCP endpoint. Review is required for approving gates and reviews, closing or deleting "
                "work, delivering results outside Maidan, and changing tokens, secrets, members or limits; reading "
                "a stored secret's value is blocked. Everything else follows your own Guard policy."
            ),
            icon="HiMiniGlobeAlt",
            launch={"kind": "remote-http", "url": f"{HOSTED}/mcp/streamable", "serverNames": ["maidan"]},
            risk_classes=risk_classes(policy, everything),
            tools=full_surface_tools(states),
            alternatives=full_alternatives,
            version=version,
        ),
        "maidan-stdio": contribution(
            slug="maidan-stdio",
            name="Maidan (local stdio)",
            description=(
                "Maidan served over stdio by the maidan binary (maidan mcp-stdio) against a local database. "
                "Same policy as the hosted endpoint: gates, removals, outside delivery and authority changes need "
                "review, reading a secret's value is blocked, and everything else follows your own Guard policy."
            ),
            icon="HiMiniCommandLine",
            launch={"kind": "direct-command", "command": "maidan"},
            risk_classes=risk_classes(policy, everything),
            tools=full_surface_tools(states),
            alternatives=full_alternatives,
            version=version,
        ),
    }
    for profile, names in sorted(load_profiles().items()):
        files[f"maidan-{profile}"] = contribution(
            slug=f"maidan-{profile}",
            name=f"Maidan ({profile} profile)",
            description=(
                f"Maidan's {profile} profile: a fixed list of the {len(names)} tools an agent needs for the "
                f"{profile} loop, served at /mcp/{profile}. Every tool is named; state changes that bind others or "
                "leave Maidan need review, and a tool the profile does not list is reviewed."
            ),
            icon="HiMiniCube",
            launch={"kind": "remote-http", "url": f"{HOSTED}/mcp/{profile}", "serverNames": [f"maidan-{profile}"]},
            risk_classes=risk_classes(policy, names) or ["maidan-own-work"],
            tools=profile_tools(names, states),
            alternatives=COMMON_ALTERNATIVES,
            version=version,
        )
    return {f"mcp.{slug}.json": body for slug, body in files.items()}


def render(body: dict) -> str:
    return json.dumps(body, indent=2) + "\n"


def print_local() -> int:
    """A custom extension can allow, which a contribution cannot: say what to allow."""
    _, states = load_policy()
    choice = {"inherit": "Allow", "review": "Recommended", "block": "Block"}
    for state in ("inherit", "review", "block"):
        names = sorted(name for name, value in states.items() if value == state)
        print(f"{choice[state]} ({len(names)}): {' '.join(names)}\n")
    print("Other tools: Recommended")
    return 0


def main() -> int:
    if "--local" in sys.argv[1:]:
        return print_local()
    check = "--check" in sys.argv[1:]
    files = build()
    stale = []
    for name, body in files.items():
        path = OUT / name
        text = render(body)
        if check:
            if not path.exists() or path.read_text() != text:
                stale.append(name)
        else:
            path.write_text(text)
    present = {path.name for path in OUT.glob("mcp.*.json")}
    for name in sorted(present - set(files)):
        if check:
            stale.append(name)
        else:
            (OUT / name).unlink()
    if stale:
        fail(f"stale HOL Guard contributions: {', '.join(sorted(stale))}; run python3 scripts/gen-hol-guard.py")
    reviewed = sum(1 for row in files["mcp.maidan.json"]["tools"] if row["state"] in REVIEWED)
    print(f"hol guard: ok ({len(files)} contributions, {reviewed} tools reviewed or blocked on the full surface)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
