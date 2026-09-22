"""The falsifiable hello-world: two agents, two languages, one lease board.

A Python worker and a TypeScript worker both call `claim_next_thread` on the same
Maidan channel. Maidan hands each open task to **exactly one** worker — no
double-claim across languages — and a claim on a drained/leased queue returns
`null`. Both workers then acknowledge, report usage, renew, and release their
fenced claims. No LLM: this is the coordination primitive, not a reasoning demo.

Run it with the orchestrator: `scripts/lease-demo.sh` (boots a server, runs this).
Or against any running Maidan:

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...        # omit only against an auth-disabled dev server
    PYTHONPATH=sdk/python/src python3 examples/lease_demo/lease_demo.py

Requires Node on PATH for the TypeScript worker (`examples/lease_demo/claim.mjs`).
"""

import json
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.request

from maidan import Client

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN")
HERE = pathlib.Path(__file__).resolve().parent


def thread_id(claim):
    """`claim_next_thread` returns a Thread object or None."""
    return claim.get("id") if isinstance(claim, dict) else None


def post(path: str, body: dict):
    """Call a lifecycle route that is outside the deliberately small SDK v1 surface."""
    headers = {"content-type": "application/json"}
    if TOKEN:
        headers["authorization"] = f"Bearer {TOKEN}"
    request = urllib.request.Request(
        f"{BASE}{path}",
        data=json.dumps(body).encode("utf-8"),
        method="POST",
        headers=headers,
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read()
            return json.loads(raw.decode("utf-8")) if raw else None
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        raise RuntimeError(f"POST {path} failed: HTTP {error.code}: {detail}") from error


def worker_result(stdout: str) -> dict:
    """Extract the tagged result without assuming it is the last output line."""
    tagged = [
        line.removeprefix("RESULT=")
        for line in stdout.splitlines()
        if line.startswith("RESULT=")
    ]
    if len(tagged) != 1:
        raise RuntimeError(f"typescript worker emitted {len(tagged)} RESULT lines")
    result = json.loads(tagged[0])
    if not isinstance(result, dict):
        raise RuntimeError("typescript worker RESULT must be a JSON object")
    return result


def main() -> int:
    c = Client(BASE, TOKEN)

    # --- setup: a workspace, two agent members, a channel, two open tasks ---
    ws = c.workspaces.create("lease-demo")["id"]
    planner = c.members.create(ws, "planner")["id"]
    reviewer = c.members.create(ws, "reviewer")["id"]
    channel = c.channels.create(ws, "coordination")["id"]
    c.threads.create(channel, "task-1: audit the login flow")
    c.threads.create(channel, "task-2: benchmark the search path")

    # --- the race: Python worker A and TypeScript worker B claim the same queue ---
    claim_a = c.claim_next_thread(channel, {"member_id": planner, "lease_secs": 120})
    claim_a_id = thread_id(claim_a)
    assert claim_a_id, "python worker should claim an open task"
    lease_a = claim_a.get("claim_lease_id")
    assert lease_a, "python claim must include its fencing token"
    print(f"[python worker]     claimed thread: {claim_a_id}")

    try:
        acknowledged = post(
            f"/threads/{claim_a_id}/claim/acknowledge",
            {"member_id": planner, "claim_lease_id": lease_a},
        )
        assert acknowledged.get("work_started_at"), "acknowledge must start the working clock"
        usage = post(f"/threads/{claim_a_id}/usage", {"tokens": 120, "turns": 1})
        assert usage.get("stopped") is False, f"unexpected budget stop: {usage}"
        renewed = c.renew_claim(claim_a_id, planner, lease_a, 300)
        assert renewed.get("assignment_expires_at"), "renew must preserve a finite lease"
        print("[python worker]     acknowledged, reported usage, renewed")

        # The TypeScript worker runs the same lifecycle, checks that a third
        # claim is null while both leases are held, then releases its own claim.
        proc = subprocess.run(
            ["node", str(HERE / "claim.mjs")],
            env={
                **os.environ,
                "MAIDAN_URL": BASE,
                "MAIDAN_CHANNEL": channel,
                "MAIDAN_MEMBER": reviewer,
            },
            capture_output=True,
            text=True,
        )
        sys.stdout.write(proc.stdout)
        if proc.returncode != 0:
            sys.stderr.write(proc.stderr)
            raise RuntimeError("typescript worker failed")
        result = worker_result(proc.stdout)
        claim_b = result.get("claim_id")

        # --- the falsifiable assertions ---
        assert claim_b, "typescript worker should claim an open task"
        assert claim_a_id != claim_b, (
            f"DOUBLE-CLAIM: python and typescript both got {claim_a_id} — the lease board failed"
        )
        assert result.get("drained") is True, "third claim must be null while both leases are held"
        assert result.get("released") is True, "typescript worker must release on clean exit"
    finally:
        released = post(
            f"/threads/{claim_a_id}/claim/release",
            {"member_id": planner, "claim_lease_id": lease_a},
        )
        assert released.get("assignee_id") is None, "release must return the task to the queue"
        print("[python worker]     released claim on exit")

    print(
        "\nOK — two distinct fenced claims, both workers acknowledged/reported/renewed, "
        "the held queue returned null, and both workers released cleanly."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
