#!/usr/bin/env python3
"""Call every tool a listing exposes, against a running Maidan.

    scripts/mcp-tool-sweep.py BASE_URL ADMIN_TOKEN

`scripts/mcp-inspector.sh` runs this after its own checks, against the
throwaway server it booted. A validated version (Open Work, Next 14) needs
every tool on a listed surface to succeed with valid parameters and to return
an actionable error with bad ones. The Inspector's own probes call one tool
per group, so this sweep adds:

1. a valid call to every tool on `/mcp/worker`, `/mcp/reviewer` and
   `/mcp/coordinator`, made on that endpoint, writes included;
2. a valid call to every read-only tool in the full catalog, on
   `/mcp/streamable`;
3. a bad call to every tool in the full catalog. A tool with required
   arguments is called with none, and its error must name one of them. Any
   other tool is called with an argument it does not declare, and its error
   must name that argument. Either call fails before the tool does anything.

The calls are plain JSON-RPC over HTTP, not the Inspector, because there are
several hundred of them and the Inspector starts a process per call. The
Inspector has already checked the same endpoints' schemas. Exits non-zero
and names every tool that failed.
"""

import json
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timezone

BASE, ADMIN = sys.argv[1].rstrip("/"), sys.argv[2]
FAILED = []
_next_id = 0


def http(method, path, body=None, bearer=ADMIN, accept="application/json"):
    data = None if body is None else json.dumps(body).encode()
    req = urllib.request.Request(BASE + path, data=data, method=method)
    req.add_header("Authorization", "Bearer " + bearer)
    req.add_header("Content-Type", "application/json")
    req.add_header("Accept", accept)
    try:
        with urllib.request.urlopen(req, timeout=60) as resp:
            raw = resp.read()
    except urllib.error.HTTPError as err:
        raw = err.read()
        if not path.startswith("/mcp"):
            sys.exit(f"FAIL: setup {method} {path}: {err.code} {raw[:300]!r}")
    return json.loads(raw) if raw else None


def rpc(endpoint, method, params, bearer=ADMIN):
    global _next_id
    _next_id += 1
    body = {"jsonrpc": "2.0", "id": _next_id, "method": method, "params": params}
    return http("POST", endpoint, body, bearer, "application/json, text/event-stream")


def call(tool, args=None, endpoint="/mcp/streamable", bearer=ADMIN):
    """(ok, text, value): the tool's answer, or the error it gave."""
    resp = rpc(endpoint, "tools/call", {"name": tool, "arguments": args or {}}, bearer)
    # The sweep makes several hundred calls on one token, near the default
    # per-token limit; a limited call waits out its window once, as a client would.
    wait = re.match(r"rate limited; retry after (\d+)ms", resp.get("error", {}).get("message", ""))
    if wait:
        time.sleep(int(wait.group(1)) / 1000 + 1)
        resp = rpc(endpoint, "tools/call", {"name": tool, "arguments": args or {}}, bearer)
    if "error" in resp:
        return False, resp["error"].get("message", ""), None
    result = resp["result"]
    text = "".join(c.get("text", "") for c in result.get("content", []))
    try:
        value = json.loads(text)
    except ValueError:
        value = None
    return result.get("isError") is not True, text, value


def must(tool, args=None, endpoint="/mcp/streamable", bearer=ADMIN):
    """A setup call: the sweep cannot go on without it."""
    ok, text, value = call(tool, args, endpoint, bearer)
    if not ok:
        sys.exit(f"FAIL: setup {tool} on {endpoint}: {text}")
    return value


def check(label, ok, detail):
    if ok:
        print(f"  ok  {label}", flush=True)
    else:
        FAILED.append(label)
        print(f"FAIL: {label}: {detail[:400]}", flush=True)


def tools_on(endpoint):
    return rpc(endpoint, "tools/list", {})["result"]["tools"]


def read_only(tool):
    return tool.get("annotations", {}).get("readOnlyHint") is True


# Setup: the bearer's workspace, a second member to review, a channel of
# fixtures for the reads, and a channel for the write flow.
RUN = uuid.uuid4().hex[:8]  # names stay unique when the sweep reruns
me = must("whoami")
ws, member = me["workspace_id"], me["member_id"]
handle = f"sweep-reviewer-{RUN}"
reviewer = http("POST", f"/workspaces/{ws}/agents", {"handle": handle})
rev_member, rev_token = reviewer["member"]["id"], reviewer["token"]["secret"]
third = http("POST", f"/workspaces/{ws}/agents", {"handle": f"sweep-third-{RUN}"})["member"]["id"]

fixtures = must("create_channel", {"workspace_id": ws, "name": f"sweep-fixtures-{RUN}"})["id"]
# Titles carry the run: a task created with the same text as an open one is
# held as its duplicate, and a claim would skip it.
thread = must("create_thread", {"channel_id": fixtures, "title": f"fixture {RUN}"})["id"]
message = must("post_message", {"thread_id": thread, "body": "fixture"})["id"]
gated = must("create_thread", {"channel_id": fixtures, "title": f"gated {RUN}"})["id"]
gate = must("request_approval", {"prompt": "fixture", "thread_id": gated})["gate_id"]
sha = must("upload_artifact", {"kind": "attachment", "content_base64": "c3dlZXA="})["sha256"]
must("create_memory_block", {"label": f"sweep-{RUN}", "value": "fixture"})
must("set_glossary_term", {"term": f"sweep-{RUN}", "definition": "a fixture"})
must("set_review_requirement", {"thread_id": thread, "required_count": 0})
group_dm = must("open_group_dm", {"workspace_id": ws, "member_ids": [member, rev_member, third]})["id"]
envelope = must("export_workspace", {"workspace_id": ws})
# Plan children name a repository, which must be on the workspace's
# github_branch allowlist.
http("POST", f"/workspaces/{ws}/egress-targets", {"surface": "github_branch", "selector": "o/r@main"})
work = must("create_channel", {"workspace_id": ws, "name": f"sweep-work-{RUN}"})["id"]
must("add_channel_member", {"channel_id": work, "member_id": rev_member})

IDS = {
    "workspace_id": ws,
    "member_id": member,
    "channel_id": fixtures,
    "thread_id": thread,
    "message_id": message,
    "gate_id": gate,
    "sha256": sha,
}
# Read-only tools whose required arguments are not one of the ids above.
READ_ARGS = {
    "advise_land_gate": {"state": {}},
    "find_thread": {"repo": "o/r", "pr_number": 1},
    "get_group_dm": {"group_dm_conversation_id": group_dm},
    "list_references": {"src_kind": "thread", "src_id": thread},
    "verify_workspace_export": {"envelope": envelope},
    "get_glossary_term": {"term": f"sweep-{RUN}"},
    "get_memory_block": {"label": f"sweep-{RUN}"},
    "wait_for_memory_block": {"label": f"sweep-{RUN}", "timeout_ms": 1},
    "get_run_occupancy": {"parent_run_id": str(uuid.uuid4())},
    "list_run_threads": {"parent_run_id": str(uuid.uuid4())},
    "parse_maidan_uri": {"uri": f"maidan://{ws}/channels/{fixtures}/threads/{thread}"},
    "search_messages": {"query": "fixture"},
    "wait_for_event": {"kinds": ["thread_landed"], "timeout_ms": 1},
}


# Tools whose answer needs a backend this throwaway server does not run, and
# the error that says so.
UNCONFIGURED = {"advise_land_gate": "resource not found"}
# verify_workspace_export also takes the envelope's fields at the top level,
# so an undeclared argument reads as part of the envelope, not as unknown.
BAD_ARGS = {"verify_workspace_export": ({"envelope": "not-an-envelope"}, ["envelope"])}


def read_args(tool):
    schema = tool["inputSchema"]
    args = {a: IDS[a] for a in schema.get("required", []) if a in IDS}
    args.update(READ_ARGS.get(tool["name"], {}))
    if "timeout_ms" in schema.get("properties", {}):
        args.setdefault("timeout_ms", 1)
    return args


def child(key):
    return {
        "key": key,
        "title": f"child {key}",
        "test": "true",
        "attributes": {"kind": "change", "acceptance": "the test passes", "engine": "session"},
        "footprint": {"repo": "o/r", "mode": "shared", "items": [{"kind": "path", "value": key}]},
    }


# The write flow: one task taken from creation through review, with a call
# to every write a profile lists. Each step names the endpoint that lists
# its tool, and a reviewer's step runs as the second member.
def write_flow():
    done = {}

    def step(tool, args, endpoint, bearer=ADMIN):
        ok, text, value = call(tool, args, endpoint, bearer)
        check(f"{tool} on {endpoint}: a valid call", ok, text)
        done[tool] = True
        return value

    co, wk, rv = "/mcp/coordinator", "/mcp/worker", "/mcp/reviewer"
    task = step("create_thread", {"channel_id": work, "title": f"sweep task {RUN}"}, co)["id"]
    step("set_thread_attributes", {"thread_id": task, "kind": "change"}, co)
    step("set_priority", {"thread_id": task, "priority": 1}, co)
    step("set_thread_steer", {"thread_id": task, "steer": "keep it small"}, co)
    step("add_reviewer", {"thread_id": task, "member_id": rev_member}, co)

    # A claim answers with the claimed thread, or null when nothing is claimable.
    claim = step("claim_next_thread", {"channel_id": work}, wk) or {}
    claimed, lease = claim.get("id"), claim.get("claim_lease_id")
    step("acknowledge_claim", {"thread_id": claimed, "claim_lease_id": lease}, wk)
    step("renew_claim", {"thread_id": claimed, "claim_lease_id": lease, "lease_secs": 600}, wk)
    step("declare_status", {"thread_id": claimed, "status": "working", "note": "sweep"}, wk)
    step("set_thread_footprint", {"thread_id": claimed, "repo": "o/r", "mode": "shared"}, wk)
    step("link_thread_artifact", {"thread_id": claimed, "sha256": sha}, wk)
    step(
        "report_usage",
        {
            "thread_id": claimed,
            "usage_report_id": str(uuid.uuid4()),
            "claim_lease_id": lease,
            "model": "sweep-model",
            "tokens": {"input": 1, "output": 1, "cache_read": 0, "cache_write_5m": 0, "cache_write_1h": 0},
            "usd_micros": 1,
            "price_snapshot": {
                f"{part}_usd_micros_per_million": 1
                for part in ("input", "output", "cache_read", "cache_write_5m", "cache_write_1h")
            },
        },
        wk,
    )
    step("set_thread_result", {"thread_id": claimed, "result": {"summary": "swept"}, "claim_lease_id": lease}, wk)
    # The mention gives the reviewer a notification to mark read below.
    step("post_message", {"thread_id": claimed, "body": f"@{handle} ready for review"}, wk)
    step("transition_thread", {"thread_id": claimed, "action": "start_review"}, wk)
    # An approval names the evidence it approves, read from the review packet.
    packet = must("get_review_packet", {"thread_id": claimed}, rv, rev_token)
    approve = {"thread_id": claimed, "decision": "approve", "evidence_root": packet["evidence_root"]}
    step("submit_review", approve, rv, rev_token)

    plan = step("create_thread", {"channel_id": work, "title": f"sweep plan {RUN}"}, co)["id"]
    step("set_thread_attributes", {"thread_id": plan, "kind": "plan"}, co)
    claim = step("claim_next_workspace_thread", {"workspace_id": ws, "kind": "plan"}, wk) or {}
    planned, lease = claim.get("id"), claim.get("claim_lease_id")
    step("submit_plan", {"thread_id": planned, "lease_id": lease, "children": [child("a")]}, wk)
    step("amend_plan", {"thread_id": planned, "lease_id": lease, "add": [child("b")]}, wk)
    step("release_claim", {"thread_id": planned, "claim_lease_id": lease}, wk)
    step("release_own_claims", {"workspace_id": ws}, wk)

    spare = step("create_thread", {"channel_id": work, "title": f"sweep cancel {RUN}"}, co)["id"]
    step("assign_thread", {"thread_id": spare, "assignee_id": member}, co)
    step("cancel_thread", {"thread_id": spare, "reason": "sweep"}, co)
    asked = step("request_approval", {"prompt": "sweep"}, wk)
    step("approval_decide", {"gate_id": asked["gate_id"], "decision": "decline"}, co, rev_token)

    notes = must("list_notifications", {"member_id": rev_member}, bearer=rev_token)
    notes = notes.get("notifications", notes) if isinstance(notes, dict) else notes
    if notes:
        step("mark_notification_read", {"member_id": rev_member, "notification_id": notes[0]["id"]}, co, rev_token)
    now = datetime.now(timezone.utc).isoformat()
    step("mark_inbox_read", {"member_id": member, "read_through": now}, co)
    return done


print("=== every profile tool: a valid call on its own endpoint ===")
written = write_flow()
profile_tools = {}
for profile in ("worker", "reviewer", "coordinator"):
    endpoint = f"/mcp/{profile}"
    for tool in tools_on(endpoint):
        profile_tools.setdefault(tool["name"], tool)
        if read_only(tool):
            ok, text, _ = call(tool["name"], read_args(tool), endpoint)
            check(f"{tool['name']} on {endpoint}: a valid call", ok, text)
missing = sorted(n for n, t in profile_tools.items() if not read_only(t) and n not in written)
check("every profile write has a step in the write flow", not missing, " ".join(missing))

print("=== every read-only tool: a valid call on /mcp/streamable ===")
catalog = tools_on("/mcp/streamable")
for tool in catalog:
    if read_only(tool):
        ok, text, _ = call(tool["name"], read_args(tool))
        expected = UNCONFIGURED.get(tool["name"])
        if expected and not ok and expected in text:
            check(f"{tool['name']}: a valid call answers {expected!r}: no backend here", True, text)
        else:
            check(f"{tool['name']}: a valid call", ok, text)

print("=== every tool: a bad call refused by name on /mcp/streamable ===")
for tool in catalog:
    required = tool["inputSchema"].get("required", [])
    args = {} if required else {"zz_not_an_argument": 1}
    named = required or ["zz_not_an_argument"]
    args, named = BAD_ARGS.get(tool["name"], (args, named))
    ok, text, _ = call(tool["name"], args)
    good = not ok and any(re.search(rf"\b{re.escape(n)}\b", text) for n in named)
    check(f"{tool['name']}: a bad call refused naming {'/'.join(named[:3])}", good, text or "it succeeded")

if FAILED:
    sys.exit(f"FAIL: {len(FAILED)} tool call(s) failed: {' '.join(FAILED[:40])}")
print(f"=== every tool call passed ({len(catalog)} tools in the catalog) ===")
