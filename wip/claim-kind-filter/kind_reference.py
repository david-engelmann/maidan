#!/usr/bin/env python3
"""Rewrite the three claim/queue tools' sections of book/src/mcp-reference.md
from the catalog, as gen-mcp-reference renders them. Run from the repo root."""
import json
import re
from pathlib import Path

catalog = {t["name"]: t for t in json.loads(Path("crates/maidan-mcp/src/tools/catalog.json").read_text())}
path = Path("book/src/mcp-reference.md")
text = path.read_text()

for name in ("claim_next_thread", "claim_next_workspace_thread", "get_workspace_queue_depth"):
    tool = catalog[name]
    head = f"### `{name}`\n\n"
    start = text.index(head)
    end = text.index("\n### ", start + len(head))
    section = text[start:end]
    title = tool["annotations"]["title"]
    new_desc = f"**{title}.** {tool['description']}\n"
    section = re.sub(r"\*\*" + re.escape(title) + r"\.\*\* [^\n]*\n", lambda _: new_desc, section, count=1)
    schema = json.dumps(tool["inputSchema"], indent=2, ensure_ascii=False)
    section = re.sub(r"```json\n.*?\n```", lambda _: f"```json\n{schema}\n```", section, count=1, flags=re.S)
    text = text[:start] + section + text[end:]

path.write_text(text)
