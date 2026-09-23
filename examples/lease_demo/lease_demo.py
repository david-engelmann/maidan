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
import uuid

from maidan import Client

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN")
WORKSPACE = os.environ.get("MAIDAN_WORKSPACE")
HERE = pathlib.Path(__file__).resolve().parent


def thread_id(claim):
    """`claim_next_thread` returns a Thread object or None."""
    return claim.get("id") if isinstance(claim, dict) else None


def post(path: str, body: dict, token: str):
    """Call a lifecycle route that is outside the deliberately small SDK v1 surface."""
    headers = {"content-type": "application/json"}
    headers["authorization"] = f"Bearer {token}"
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
    admin = Client(BASE, TOKEN)

    # --- setup: two member-bound workers, one channel, two open tasks ---
    assert WORKSPACE, "MAIDAN_WORKSPACE is required"
    planner = admin.members.create(WORKSPACE, "planner")["id"]
    reviewer = admin.members.create(WORKSPACE, "reviewer")["id"]
    worker_caps = ["workspace:read", "thread:transition"]
    planner_token = admin.tokens.mint(WORKSPACE, planner, worker_caps)["secret"]
    reviewer_token = admin.tokens.mint(WORKSPACE, reviewer, worker_caps)["secret"]
    planner_client = Client(BASE, planner_token)
    channel = admin.channels.create(WORKSPACE, "coordination")["id"]
    admin.threads.create(channel, "task-1: audit the login flow")
    admin.threads.create(channel, "task-2: benchmark the search path")

    # --- the race: Python worker A and TypeScript worker B claim the same queue ---
    claim_a = planner_client.claim_next_thread(channel, {"lease_secs": 120})
    claim_a_id = thread_id(claim_a)
    assert claim_a_id, "python worker should claim an open task"
    lease_a = claim_a.get("claim_lease_id")
    assert lease_a, "python claim must include its fencing token"
    print(f"[python worker]     claimed thread: {claim_a_id}")

    try:
        acknowledged = post(
            f"/threads/{claim_a_id}/claim/acknowledge",
            {"claim_lease_id": lease_a},
            planner_token,
        )
        assert acknowledged.get("work_started_at"), "acknowledge must start the working clock"
        usage = post(
            f"/threads/{claim_a_id}/usage",
            {
                "usage_report_id": str(uuid.uuid4()),
                "claim_lease_id": lease_a,
                "model": "demo-model",
                "tokens": {"input": 120, "output": 0, "cache_read": 0, "cache_write": 0},
                "usd_micros": 0,
                "price_snapshot": {
                    "input_usd_micros_per_million": 0,
                    "output_usd_micros_per_million": 0,
                    "cache_read_usd_micros_per_million": 0,
                    "cache_write_usd_micros_per_million": 0,
                },
                "turns": 1,
            },
            planner_token,
        )
        assert usage.get("stopped") is False, f"unexpected budget stop: {usage}"
        renewed = planner_client.renew_claim(claim_a_id, lease_a, 300)
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
                "MAIDAN_TOKEN": reviewer_token,
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
            {"claim_lease_id": lease_a},
            planner_token,
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
