#!/usr/bin/env python3
"""Rewrite every file that copies the MCP catalog, from the checked-in catalog
(crates/maidan-mcp/src/tools/catalog.json, which the server parses as tools/list).

The tool names contract, the Docker and ToolHive listings, and the tool counts
in docs/Protocols.md, README.md and book/src/introduction.md are copies of the
catalog. Two PRs that each add a tool conflict in all of them; resolving takes
either side of catalog.json and re-running this (scripts/regen.sh runs it).
"""
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
tools = json.loads((ROOT / "crates/maidan-mcp/src/tools/catalog.json").read_text())
names = [t["name"] for t in tools]


def write_json(path, value, sort_keys=False):
    (ROOT / path).write_text(json.dumps(value, indent=2, ensure_ascii=False, sort_keys=sort_keys) + "\n")


write_json("contracts/mcp-tool-names.json", sorted(names))

docker = json.loads((ROOT / "listings/docker/tools.json").read_text())
docker["tools"] = [{"name": t["name"], "description": t["description"]} for t in tools]
write_json("listings/docker/tools.json", docker, sort_keys=True)

toolhive = json.loads((ROOT / "listings/toolhive/server.json").read_text())
toolhive["tools"] = names
write_json("listings/toolhive/server.json", toolhive)

count = len(names)
for path, pattern, repl in [
    ("docs/Protocols.md", r"MCP tool count is \*\*\d+\*\*", f"MCP tool count is **{count}**"),
    ("README.md", r"// \d+ tools in tools/list", f"// {count} tools in tools/list"),
    ("book/src/introduction.md", r"// \d+ tools in tools/list", f"// {count} tools in tools/list"),
]:
    text = (ROOT / path).read_text()
    new, n = re.subn(pattern, repl, text)
    if n == 0:
        sys.exit(f"regen: {path} no longer states the tool count as `{pattern}`")
    (ROOT / path).write_text(new)

print(f"regen: {count} tools written to the names contract, both listings and three docs")
