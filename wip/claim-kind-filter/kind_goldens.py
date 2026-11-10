#!/usr/bin/env python3
"""Add the `kind` filter to the MCP catalog copies and the OpenAPI golden.

Run from the repository root. Each file is rewritten in the exact format it
was read in (checked by a round trip before editing).
"""
import json
import sys
from pathlib import Path

CLAIM_KIND = (
    "only claim a thread of this kind (change, plan or review; a thread that "
    "names no kind is a change). A review seat claims with kind review and its "
    "repo, and no engine, to take every review in that repository whatever "
    "engine wrote the change. Combines with repo, engine and lane. Omitted, any kind"
)
DEPTH_KIND = (
    "count only threads of this kind (change, plan or review; a thread that "
    "names no kind is a change), as claim_next_workspace_thread's kind. "
    "Omitted, any kind"
)
KIND_ENUM = ["change", "plan", "review"]

OLD_DEPTH_DESC = "repo and engine narrow the count as they narrow claim_next_workspace_thread."
NEW_DEPTH_DESC = "repo, engine and kind narrow the count as they narrow claim_next_workspace_thread."


def kind_prop(description):
    return {"description": description, "enum": KIND_ENUM, "type": "string"}


def sorted_props(props):
    return dict(sorted(props.items()))


def patch_tools(tools):
    hit = set()
    for tool in tools:
        name = tool.get("name")
        props = tool.get("inputSchema", {}).get("properties")
        if name in ("claim_next_thread", "claim_next_workspace_thread"):
            props["kind"] = kind_prop(CLAIM_KIND)
            tool["inputSchema"]["properties"] = sorted_props(props)
            hit.add(name)
        elif name == "get_workspace_queue_depth":
            props["kind"] = kind_prop(DEPTH_KIND)
            tool["inputSchema"]["properties"] = sorted_props(props)
            assert OLD_DEPTH_DESC in tool["description"], tool["description"]
            tool["description"] = tool["description"].replace(OLD_DEPTH_DESC, NEW_DEPTH_DESC)
            hit.add(name)
    return hit


def pretty(value):
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def compact(value):
    return json.dumps(value, separators=(",", ":"), ensure_ascii=False)


def rewrite(path, dump, edit):
    text = Path(path).read_text()
    value = json.loads(text)
    assert dump(value) == text, f"{path}: format does not round-trip"
    edit(value)
    Path(path).write_text(dump(value))


def tools_of(value):
    return value["tools"] if isinstance(value, dict) and "tools" in value else value


def edit_catalog(value):
    hit = patch_tools(tools_of(value))
    print("catalog-like", sorted(hit))


def find_tools(value):
    """The tool list inside a JSON-RPC tools/list snapshot, wherever it sits."""
    if isinstance(value, list) and value and isinstance(value[0], dict) and "inputSchema" in value[0]:
        return value
    if isinstance(value, dict):
        for v in value.values():
            found = find_tools(v)
            if found is not None:
                return found
    if isinstance(value, list):
        for v in value:
            found = find_tools(v)
            if found is not None:
                return found
    return None


def edit_snapshot(value):
    hit = patch_tools(find_tools(value))
    print("snapshot", sorted(hit))


def edit_openapi(value, claim_doc, depth_doc, depth_response):
    schema = value["components"]["schemas"]["ClaimNextThread"]
    schema["properties"]["kind"] = {
        "oneOf": [
            {"type": "null"},
            {"$ref": "#/components/schemas/TaskKind", "description": claim_doc},
        ]
    }
    schema["properties"] = sorted_props(schema["properties"])
    get = value["paths"]["/workspaces/{wid}/queue-depth"]["get"]
    get["parameters"].append(
        {
            "description": depth_doc,
            "in": "query",
            "name": "kind",
            "required": False,
            "schema": {"$ref": "#/components/schemas/TaskKind"},
        }
    )
    get["responses"]["200"]["description"] = depth_response


if __name__ == "__main__":
    which = sys.argv[1]
    if which == "mcp":
        rewrite("crates/maidan-mcp/src/tools/catalog.json", pretty, edit_catalog)
        rewrite("crates/maidan-mcp/tests/fixtures/wire/tools_list_full.json", pretty, edit_snapshot)
        rewrite("crates/maidan-mcp/tests/fixtures/wire/tools_list_worker.json", pretty, edit_snapshot)
        rewrite("crates/maidan-mcp/tests/fixtures/profile-worker.json", compact, edit_catalog)
    elif which == "openapi":
        docs = json.loads(Path(sys.argv[2]).read_text())
        rewrite(
            "crates/maidan-server/tests/fixtures/wire/openapi.json",
            pretty,
            lambda v: edit_openapi(v, docs["claim"], docs["depth"], docs["depth_response"]),
        )
