#!/usr/bin/env python3
"""Call every tool a listing exposes, against a running Maidan.

    scripts/mcp-tool-sweep.py BASE_URL ADMIN_TOKEN [--without s3]

`scripts/mcp-inspector.sh` runs this after its own checks, against the
throwaway server it booted. A validated version (Open Work, Next 14) needs
every tool on a listed surface to succeed with valid parameters and to return
an actionable error with bad ones. The Inspector's own probes call one tool
per group, so this sweep adds:

1. a valid call to every tool on `/mcp/worker`, `/mcp/reviewer` and
   `/mcp/coordinator`, made on that endpoint, writes included;
2. a valid call to every other write in the full catalog, on
   `/mcp/streamable`;
3. a valid call to every read-only tool in the full catalog, on
   `/mcp/streamable`;
4. a bad call to every tool in the full catalog. A tool with required
   arguments is called with none, and its error must name one of them. Any
   other tool is called with an argument it does not declare, and its error
   must name that argument. Either call fails before the tool does anything.

Six tools need a backend beyond the store (`BACKENDS`): the four multipart
artifact tools need S3, `top_up_credits` a credits receiving address, and
`advise_land_gate` a land-gate advisor. The inspector runs all three (MinIO,
a test address on Base Sepolia, a stub advisor), and each tool must succeed
with the answer a real backend gives: the multipart flow stores an artifact
whose bytes read back and aborts a second upload, the top-up opens an intent,
the advisor recommends a land. `--without s3` says the server was booted
without S3 (the inspector passes it when no Docker daemon answers): its tools
must then answer the error naming S3, and the summary says they were NOT
exercised. A success there fails, since the flag was wrong.

The calls are plain JSON-RPC over HTTP, not the Inspector, because there are
several hundred of them and the Inspector starts a process per call. The
Inspector has already checked the same endpoints' schemas. Exits non-zero
and names every tool that failed.
"""

import argparse
import base64
import hashlib
import json
import re
import sys
import time
import urllib.error
import urllib.request
import uuid
from datetime import datetime, timedelta, timezone

# The backends a tool needs beyond the store, the tools that need each, and
# the error each tool answers when its backend is missing.
BACKENDS = {
    "s3": (
        "S3 artifact backend",
        [
            "begin_artifact_multipart",
            "upload_artifact_multipart_part",
            "complete_artifact_multipart",
            "abort_artifact_multipart",
        ],
    ),
    "credits": ("MAIDAN_CREDITS_ADDRESS is not set", ["top_up_credits"]),
    "land-gate-advisor": ("resource not found", ["advise_land_gate"]),
}
NEEDS_BACKEND = {tool: backend for backend, (_, tools) in BACKENDS.items() for tool in tools}
# What a real backend's answer holds, so a call that succeeds with nothing in
# it does not pass for one. The multipart flow checks its own answers.
REAL_ANSWER = {
    "top_up_credits": lambda v: v.get("status") == "open"
    and str(v.get("eip681", "")).startswith("ethereum:")
    and 5_000_000 < v.get("amount_micros", 0) < 5_010_000,
    "advise_land_gate": lambda v: v.get("provider") == "typesafe"
    and v.get("recommended_land") in ("green", "amber", "red"),
}

parser = argparse.ArgumentParser(description="Call every MCP tool on a running Maidan.")
parser.add_argument("base_url")
parser.add_argument("admin_token")
parser.add_argument(
    "--without",
    action="append",
    default=[],
    choices=["s3"],
    help="a backend the server was booted without; its tools must answer the error naming it",
)
OPTS = parser.parse_args()
BASE, ADMIN, WITHOUT = OPTS.base_url.rstrip("/"), OPTS.admin_token, set(OPTS.without)
FAILED = []
# Each backend tool's outcome: "real" when it succeeded with its backend
# configured, "not exercised" when its backend was absent and it said so.
BACKEND_OUTCOME = {}
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
    # A client shows the model only `content`. The multipart tools answered a
    # bare object, which read here as a success with nothing in it.
    if not isinstance(result.get("content"), list):
        return False, f"the result carries no content: {json.dumps(result)[:300]}", None
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


def valid(tool, ok, text, value, label=None):
    """Judge a valid call. A tool whose backend this run is without must
    answer the error naming it; any other must succeed, and a backend tool
    with what a real backend answers."""
    label = label or f"{tool}: a valid call"
    backend = NEEDS_BACKEND.get(tool)
    if backend in WITHOUT:
        error = BACKENDS[backend][0]
        held = not ok and error in text
        check(f"{tool}: no {backend} here, so a valid call answers {error!r}", held, text or "it succeeded")
        if held:
            BACKEND_OUTCOME.setdefault(tool, "not exercised")
        return
    real = REAL_ANSWER.get(tool)
    if ok and real and not (isinstance(value, dict) and real(value)):
        check(f"{label}, {backend} configured, answered as the real one answers", False, text)
        return
    check(f"{label}, {backend} configured" if backend else label, ok, text)
    if backend and ok:
        BACKEND_OUTCOME.setdefault(tool, "real")


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
# The ChatGPT listing reaches /mcp/coordinator through an OAuth grant, which
# carries only the delegatable capabilities. The coordinator's calls run on a
# token of the bearer's holding exactly those, so a tool that works for an
# admin but not for a grant fails here, not in a directory's review.
OAUTH_GRANT_CAPABILITIES = [
    "workspace:read",
    "workspace:write",
    "member:self",
    "message:post",
    "thread:transition",
    "artifact:upload",
    "search:query",
    "event:subscribe",
]
GRANT_NOTE = ", with only an OAuth grant's capabilities"
grant_token = http(
    "POST", f"/workspaces/{ws}/members/{member}/tokens", {"capabilities": OAUTH_GRANT_CAPABILITIES}
)["secret"]

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
    "advise_land_gate": {"state": {"title": f"fixture {RUN}", "tests": "pass"}},
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


DELIVERED = []


# The write flow: one task taken from creation through review, with a call
# to every write a profile lists. Each step names the endpoint that lists
# its tool, and a reviewer's step runs as the second member.
def write_flow():
    done = {}
    co, wk, rv = "/mcp/coordinator", "/mcp/worker", "/mcp/reviewer"

    def step(tool, args, endpoint, bearer=None):
        bearer = bearer or (grant_token if endpoint == co else ADMIN)
        ok, text, value = call(tool, args, endpoint, bearer)
        check(f"{tool} on {endpoint}: a valid call{GRANT_NOTE if bearer == grant_token else ''}", ok, text)
        done[tool] = True
        return value

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
    # A result routed to a Slack channel arms a delivery, which the catalog
    # flow replays.
    result = {
        "schema": "maidan.waiter.result/1",
        "result_kind": "example.sweep/1",
        "status": "reviewed",
        "summary": "swept",
        "deliver_to": [{"surface": "slack", "channel": "C0SWEEP"}],
    }
    step("set_thread_result", {"thread_id": claimed, "result": result, "claim_lease_id": lease}, wk)
    DELIVERED.append(claimed)
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


# Every other write in the full catalog, on /mcp/streamable, in an order where
# each call has what it needs: a write and the one that undoes it sit side by
# side, and the calls that spend a token or a member come last.
def catalog_flow():
    done = {}
    later = (datetime.now(timezone.utc) + timedelta(days=1)).isoformat()

    def w(tool, args, bearer=ADMIN):
        ok, text, value = call(tool, args, bearer=bearer)
        valid(tool, ok, text, value)
        done[tool] = True
        return value if isinstance(value, (dict, list)) else {}

    new = lambda title: must("create_thread", {"channel_id": work, "title": f"{title} {RUN}"})["id"]
    t, other = new("catalog"), new("catalog other")
    m = must("post_message", {"thread_id": t, "body": "catalog"})["id"]

    w("set_workspace_handle", {"workspace_id": ws, "handle": f"sweep-{RUN}"})
    ch = w("create_channel", {"workspace_id": ws, "name": f"sweep-catalog-{RUN}"})["id"]
    w("add_channel_member", {"channel_id": ch, "member_id": third})
    w("remove_channel_member", {"channel_id": ch, "member_id": third})
    for kind, key, value in (("thread", "thread_id", t), ("channel", "channel_id", ch)):
        w(f"mute_{kind}", {key: value})
        w(f"unmute_{kind}", {key: value})
    budget = {"max_tokens": 1000, "max_usd_micros": 1000, "max_turns": 10, "max_wall_secs": 600}
    w("set_thread_budget", {"thread_id": t, **budget})
    w("update_thread_budget", {"thread_id": t, "max_turns": 20})
    w("rename_thread", {"thread_id": t, "title": f"catalog renamed {RUN}"})
    w("set_thread_owner", {"thread_id": t, "owner_id": member})
    w("set_wait", {"thread_id": t, "wait_until": later})
    w("cancel_wait", {"thread_id": t})
    w("mark_unclaimable", {"thread_id": t, "reason": "sweep"})
    w("mark_claimable", {"thread_id": t})
    w("set_thread_block", {"thread_id": t, "reason": "human"})
    w("clear_thread_block", {"thread_id": t})
    w("add_thread_dependency", {"thread_id": t, "depends_on_thread_id": other})
    w("remove_thread_dependency", {"thread_id": t, "depends_on_thread_id": other})
    w("set_thread_lineage", {"thread_id": t, "parent_run_id": str(uuid.uuid4())})
    w("clear_thread_lineage", {"thread_id": t})
    w("add_thread_required_skill", {"thread_id": t, "skill": "rust"})
    w("remove_thread_required_skill", {"thread_id": t, "skill": "rust"})
    # Recording a land gate needs the land_gate skill.
    w("add_member_skill", {"member_id": member, "skill": "land_gate"})
    w("set_review_requirement", {"thread_id": t, "required_count": 1})
    w("add_reviewer", {"thread_id": t, "member_id": rev_member})
    w("remove_reviewer", {"thread_id": t, "member_id": rev_member})
    w("clear_review_requirement", {"thread_id": t})
    w("require_land_gate", {"thread_id": t})
    w("set_land_gate", {"thread_id": t, "status": "pass"})
    w("clear_land_gate", {"thread_id": t})
    w("remove_member_skill", {"member_id": member, "skill": "land_gate"})
    w("add_evidence_record", {"thread_id": t, "kind": "note", "source": "sweep", "note": "swept"})
    w("link_thread_artifact", {"thread_id": t, "sha256": sha})
    w("unlink_thread_artifact", {"thread_id": t, "sha256": sha})
    w("snapshot_thread_context", {"thread_id": t})
    w("claim_thread", {"thread_id": t})
    w("unassign_thread", {"thread_id": t})
    w("close_abandoned_thread", {"thread_id": other, "reason": "abandoned"})

    w("edit_message", {"message_id": m, "body": "catalog, edited"})
    w("record_mention", {"message_id": m, "member_id": member})
    for vote in ("cast_vote", "retract_vote"):
        w(vote, {"message_id": m, "kind": "ack"})
    w("add_reaction", {"message_id": m, "emoji": "eyes"})
    w("remove_reaction", {"message_id": m, "emoji": "eyes"})
    w("pin_message", {"thread_id": t, "message_id": m})
    w("unpin_message", {"thread_id": t, "message_id": m})
    w("add_reference", {"src_kind": "thread", "src_id": t, "dst_kind": "message", "dst_id": m, "relation": "cites"})
    w("seed_from_message", {"message_id": m, "title": f"seeded {RUN}"})
    w("tombstone_message", {"message_id": must("post_message", {"thread_id": t, "body": "gone"})["id"]})

    dm = w("open_dm_conversation", {"workspace_id": ws, "other_member_id": rev_member})["id"]
    w("post_dm_message", {"dm_conversation_id": dm, "body": "hello"})
    gdm = w("open_group_dm", {"workspace_id": ws, "member_ids": [member, rev_member, third]})["id"]
    w("post_group_dm_message", {"group_dm_conversation_id": gdm, "body": "hello"})

    notes = must("list_notifications", {"member_id": member})
    notes = notes.get("notifications", notes) if isinstance(notes, dict) else notes
    note = notes[0]["id"] if notes else str(uuid.uuid4())
    w("snooze_notification", {"member_id": member, "notification_id": note, "until": later})
    w("mark_all_notifications_read", {"member_id": member})
    w("set_notification_pref", {"member_id": member, "kind": "mention_recorded", "muted": False})
    w("set_delivery_mode", {"member_id": member, "mode": "immediate"})
    w("set_member_email", {"member_id": member, "email": f"sweep-{RUN}@example.com"})
    w("delete_member_email", {"member_id": member})
    for kind, key, value in (("channel", "channel_id", ch), ("thread", "thread_id", t), ("member", "followed_member_id", rev_member)):
        w(f"follow_{kind}", {"member_id": member, key: value})
        w(f"unfollow_{kind}", {"member_id": member, key: value})

    w("set_wip_limit", {"limit": 5})
    w("set_delegation_policy", {"max_grant_days": 30})
    w("set_retention_policy", {"messages_days": 3650})
    w("set_spawn_budget", {"max_depth": 3})
    block_id = w("create_memory_block", {"label": f"catalog-{RUN}", "value": "one"})["id"]
    w("set_memory_block_value", {"label": f"catalog-{RUN}", "value": "two", "expected_revision": 1})
    w("attach_memory_block", {"thread_id": t, "label": f"catalog-{RUN}"})
    w("detach_memory_block", {"thread_id": t, "block_id": block_id})
    w("delete_memory_block", {"block_id": block_id})
    w("set_glossary_term", {"term": f"catalog-{RUN}", "definition": "a fixture"})
    w("delete_glossary_term", {"term": f"catalog-{RUN}"})
    w("create_secret", {"name": f"SWEEP_{RUN.upper()}", "value": "s3cret"})
    w("resolve_secret", {"name": f"SWEEP_{RUN.upper()}"})
    w("delete_secret", {"name": f"SWEEP_{RUN.upper()}"})
    w("allow_secret_egress_host", {"host": "hooks.example.com"})
    w("revoke_secret_egress_host", {"host": "hooks.example.com"})
    ticket = w("create_share_ticket", {"channel_id": ch, "expires_at": later})
    w("revoke_share_ticket", {"ticket_id": ticket.get("ticket", {}).get("id")})
    spec = {"children": [{"key": "a", "title": "child"}]}
    recipe = w("create_recipe", {"channel_id": ch, "name": f"recipe-{RUN}", "spec": spec})["id"]
    w("instantiate_recipe", {"recipe_id": recipe})
    schedule = w("create_task_schedule", {"channel_id": ch, "title": f"scheduled {RUN}"})["id"]
    w("set_task_schedule_active", {"schedule_id": schedule, "active": False})
    w("delete_task_schedule", {"schedule_id": schedule})
    w("delete_recipe", {"recipe_id": recipe})
    target = {"handler_kind": "mcp_tool", "handler_target": "whoami"}
    command = w("register_slash_command", {"workspace_id": ws, "name": f"sweep{RUN}", **target})
    w("revoke_slash_command", {"workspace_id": ws, "command_id": command.get("id") or command.get("command_id")})
    hook = w("register_fsm_hook", {"workspace_id": ws, **target})
    w("revoke_fsm_hook", {"workspace_id": ws, "hook_id": hook.get("id") or hook.get("hook_id")})
    w("link_github_issue", {"thread_id": t, "repo": "o/r", "issue_number": 1})
    w("unlink_github_issue", {"repo": "o/r", "issue_number": 1})
    w("link_slack_channel", {"thread_id": t, "slack_channel_id": "C0SWEEP"})
    w("unlink_slack_channel", {"slack_channel_id": "C0SWEEP"})
    w("upload_artifact", {"kind": "attachment", "content_base64": "Y2F0YWxvZw=="})
    # One upload taken to a stored artifact, and a second begun and aborted.
    # Without S3 nothing is begun, so ids in the shape the tools check stand in.
    def begin():
        begun = w("begin_artifact_multipart", {})
        return {
            "upload_id": begun.get("upload_id", "sweep"),
            "object_key": begun.get("object_key", f"multipart/{ws}/{uuid.uuid4()}"),
        }

    content = f"multipart {RUN}".encode()
    upload = begin()
    part = {"part_number": 1, "content_base64": base64.b64encode(content).decode()}
    etag = w("upload_artifact_multipart_part", {**upload, **part}).get("etag", "sweep")
    parts = [{"part_number": 1, "etag": etag}]
    stored = w("complete_artifact_multipart", {**upload, "parts": parts, "kind": "attachment"})
    aborted = begin()
    w("abort_artifact_multipart", aborted)
    if "s3" not in WITHOUT:
        digest = hashlib.sha256(content).hexdigest()
        ok, text, value = call("get_artifact", {"sha256": stored.get("sha256", "")})
        got = (value or {}).get("content_base64", "") if ok else ""
        check(
            "complete_artifact_multipart: the artifact reads back as the part's bytes",
            stored.get("sha256") == digest and base64.b64decode(got) == content,
            f"completed {json.dumps(stored)[:200]}; read {text[:200]}",
        )
        ok, text, _ = call("upload_artifact_multipart_part", {**aborted, **part})
        check("abort_artifact_multipart: the aborted upload takes no part", not ok, text or "it took one")
    w("watch_pull", {"workspace_id": ws, "repo": "o/r", "pull_number": 1})
    w("register_thread_pull", {"thread_id": t, "repo": "o/r", "pull_number": 2})
    delivered = DELIVERED[0] if DELIVERED else t
    deliveries = must("list_result_deliveries", {"thread_id": delivered})
    deliveries = deliveries.get("deliveries", deliveries) if isinstance(deliveries, dict) else deliveries
    delivery = deliveries[0]["id"] if deliveries else str(uuid.uuid4())
    w("replay_result_delivery", {"thread_id": delivered, "delivery_id": delivery})
    w("top_up_credits", {"workspace_id": ws, "usd": 5})
    w("export_workspace", {"workspace_id": ws})
    w("import_workspace", {"envelope": envelope})

    grant = w(
        "create_delegation_grant",
        {
            "workspace_id": ws,
            "subject_id": member,
            "delegate_id": rev_member,
            "capabilities": ["workspace:read"],
            "purpose": "sweep",
            "expires_at": later,
        },
    )
    grant_id = grant.get("id") or grant.get("grant_id")
    w("delegate_token", {"grant_id": grant_id}, rev_token)
    w("revoke_delegation_grant", {"workspace_id": ws, "grant_id": grant_id})
    w("attenuate_token", {"capabilities": ["workspace:read"]})
    w("freeze_member", {"member_id": third, "reason": "sweep"})
    w("unfreeze_member", {"member_id": third})
    # rotate_token retires the token that calls it, so it spends a token of
    # the third member's, minted for it.
    spare = http("POST", f"/workspaces/{ws}/members/{third}/tokens", {"capabilities": ["workspace:read"]})
    w("rotate_token", {}, spare["secret"])
    return done


print("=== every profile tool: a valid call on its own endpoint ===")
written = write_flow()
profile_tools = {}
for profile in ("worker", "reviewer", "coordinator"):
    endpoint = f"/mcp/{profile}"
    for tool in tools_on(endpoint):
        profile_tools.setdefault(tool["name"], tool)
        if read_only(tool):
            bearer = grant_token if profile == "coordinator" else ADMIN
            ok, text, value = call(tool["name"], read_args(tool), endpoint, bearer)
            label = f"{tool['name']} on {endpoint}: a valid call{GRANT_NOTE if bearer == grant_token else ''}"
            valid(tool["name"], ok, text, value, label)
missing = sorted(n for n, t in profile_tools.items() if not read_only(t) and n not in written)
check("every profile write has a step in the write flow", not missing, " ".join(missing))

print("=== every other write: a valid call on /mcp/streamable ===")
catalog = tools_on("/mcp/streamable")
written.update(catalog_flow())
missing = sorted(t["name"] for t in catalog if not read_only(t) and t["name"] not in written)
check("every write in the catalog has a step in a write flow", not missing, " ".join(missing))

print("=== every read-only tool: a valid call on /mcp/streamable ===")
for tool in catalog:
    if read_only(tool):
        ok, text, value = call(tool["name"], read_args(tool))
        valid(tool["name"], ok, text, value)

print("=== every tool: a bad call refused by name on /mcp/streamable ===")
for tool in catalog:
    required = tool["inputSchema"].get("required", [])
    args = {} if required else {"zz_not_an_argument": 1}
    named = required or ["zz_not_an_argument"]
    args, named = BAD_ARGS.get(tool["name"], (args, named))
    ok, text, _ = call(tool["name"], args)
    good = not ok and any(re.search(rf"\b{re.escape(n)}\b", text) for n in named)
    check(f"{tool['name']}: a bad call refused naming {'/'.join(named[:3])}", good, text or "it succeeded")

# A backend tool the catalog lost, or one no flow called, would otherwise
# vanish from the summary instead of failing it.
listed = {t["name"] for t in catalog}
for tool in NEEDS_BACKEND:
    check(f"{tool}: listed, and a valid call passed", tool in listed and tool in BACKEND_OUTCOME, "none did")

if FAILED:
    sys.exit(f"FAIL: {len(FAILED)} tool call(s) failed: {' '.join(FAILED[:40])}")
print("=== the tools that need a backend beyond the store ===")
for tool, backend in NEEDS_BACKEND.items():
    if BACKEND_OUTCOME[tool] == "real":
        print(f"  ok  {tool}: succeeded, {backend} configured")
    else:
        print(f"  NOT EXERCISED  {tool}: no {backend} here; it answered only {BACKENDS[backend][0]!r}")
if WITHOUT:
    print(
        f"=== every tool call passed ({len(catalog)} tools in the catalog), but "
        f"{', '.join(sorted(WITHOUT))} was NOT exercised ==="
    )
else:
    print(
        f"=== every tool call passed ({len(catalog)} tools in the catalog), "
        f"the {len(NEEDS_BACKEND)} that need a backend included ==="
    )
