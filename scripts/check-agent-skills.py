#!/usr/bin/env python3
"""Check the agent skills in examples/claude-code against the MCP catalog.

A skill that names a tool the server no longer has sends an agent into a
refused call, and a tool no skill mentions is one no agent learns to use. So
every skill must:

- carry Agent Skills frontmatter whose name is its directory's;
- end with a "## Tools" list of catalog tools, each also used in its text;
- name no tool-shaped word in its files that the catalog does not have;

and together the skills must cover the whole catalog, apart from the tools
listed in NOT_IN_A_SKILL with the reason.
"""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SKILLS = ROOT / "examples" / "claude-code"
CATALOG = ROOT / "crates" / "maidan-mcp" / "src" / "tools" / "catalog.json"

# Tools no skill teaches, each with why.
NOT_IN_A_SKILL: dict[str, str] = {}

# Backticked snake_case words that start like a tool name but are fields,
# values or REST paths, not tools.
NOT_TOOLS = {
    "claim_lost",
}

# A field or value named after a tool's verb (claim_lease_id, wait_ms).
FIELD_SUFFIXES = ("_id", "_ids", "_ms", "_secs", "_at", "_until")

NAME = re.compile(r"^[a-z0-9]+(-[a-z0-9]+)*$")
TICKED = re.compile(r"`([a-z][a-z0-9]*(?:_[a-z0-9]+)*)`")


def skill_under_root(skill: Path, skills_root: Path) -> bool:
    """True when ``skill`` resolves to a direct child directory of ``skills_root``."""
    try:
        resolved = skill.resolve()
        root = skills_root.resolve()
    except OSError:
        return False
    return resolved.is_dir() and resolved.parent == root and resolved.name == skill.name


def frontmatter(text: str) -> dict[str, str]:
    if not text.startswith("---\n"):
        return {}
    end = text.find("\n---\n", 4)
    if end < 0:
        return {}
    fields = {}
    for line in text[4:end].splitlines():
        key, sep, value = line.partition(":")
        if sep:
            fields[key.strip()] = value.strip()
    return fields


def main() -> int:
    catalog = {tool["name"] for tool in json.loads(CATALOG.read_text())}
    verbs = {name.split("_")[0] for name in catalog} | {"wait"}
    errors: list[str] = []
    covered: set[str] = set()

    skills_root = SKILLS.resolve()
    skills = sorted(path.parent for path in SKILLS.glob("*/SKILL.md"))
    if not skills:
        errors.append(f"no skills under {SKILLS.relative_to(ROOT)}")
    for skill in skills:
        try:
            skill_resolved = skill.resolve()
        except OSError as exc:
            errors.append(f"{skill}: cannot resolve skill path ({exc})")
            continue
        if skill_resolved != skills_root / skill_resolved.name or not skill_resolved.is_dir():
            errors.append(f"{skill}: skill path escapes {SKILLS.relative_to(ROOT)}")
            continue
        where = skill_resolved.relative_to(ROOT)
        text = (skill_resolved / "SKILL.md").read_text()
        meta = frontmatter(text)
        if meta.get("name") != skill_resolved.name:
            errors.append(f"{where}: frontmatter name must be {skill_resolved.name!r}")
        if not NAME.match(skill_resolved.name) or len(skill_resolved.name) > 64:
            errors.append(f"{where}: a skill name is 1 to 64 lowercase letters, digits and hyphens")
        description = meta.get("description", "").strip()
        if not description or len(description) > 1024:
            errors.append(f"{where}: description must be 1 to 1024 characters")
        skill = skill_resolved

        body, sep, tools_section = text.partition("\n## Tools\n")
        if not sep:
            errors.append(f"{where}: no '## Tools' section")
            continue
        listed = set(TICKED.findall(tools_section))
        for name in sorted(listed - catalog):
            errors.append(f"{where}: Tools lists {name}, which the catalog does not have")
        # The text is the skill's own body plus the runbooks beside it.
        docs = sorted(skill.glob("*.md"))
        used = set(TICKED.findall(body))
        for doc in docs:
            try:
                doc_resolved = doc.resolve()
            except OSError:
                errors.append(f"{where}: cannot resolve {doc.name}")
                continue
            if not doc_resolved.is_relative_to(skill):
                errors.append(f"{where}: {doc.name} escapes the skill directory")
                continue
            if doc_resolved.name != "SKILL.md":
                used |= set(TICKED.findall(doc_resolved.read_text()))
        for name in sorted((listed & catalog) - used):
            errors.append(f"{where}: Tools lists {name} but the skill never says how to use it")
        covered |= listed & catalog

        for doc in docs:
            try:
                doc_resolved = doc.resolve()
            except OSError:
                continue
            if not doc_resolved.is_relative_to(skill):
                continue
            for name in sorted(set(TICKED.findall(doc_resolved.read_text()))):
                if name in catalog or name in NOT_TOOLS or "_" not in name:
                    continue
                if name.endswith(FIELD_SUFFIXES):
                    continue
                if name.split("_")[0] in verbs:
                    errors.append(f"{doc_resolved.relative_to(ROOT)}: `{name}` looks like a tool and is not in the catalog")

    for name in sorted(set(NOT_IN_A_SKILL) - catalog):
        errors.append(f"NOT_IN_A_SKILL names {name}, which the catalog does not have")
    for name in sorted(catalog - covered - set(NOT_IN_A_SKILL)):
        errors.append(f"no skill covers {name}: add it to the skill whose flow uses it")

    for error in errors:
        print(f"FAIL: {error}")
    if errors:
        return 1
    print(f"agent skills: ok ({len(skills)} skills cover {len(covered)} of {len(catalog)} tools)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
