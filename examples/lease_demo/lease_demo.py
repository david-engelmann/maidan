"""The falsifiable hello-world: two agents, two languages, one lease board.

A Python worker and a TypeScript worker both call `claim_next_thread` on the same
Maidan channel. Maidan hands each open task to **exactly one** worker — no
double-claim across languages — and a claim on a drained/leased queue returns
`null`. No LLM: this is the coordination primitive, not a reasoning demo.

Run it with the orchestrator: `scripts/lease-demo.sh` (boots a server, runs this).
Or against any running Maidan:

    export MAIDAN_URL=http://127.0.0.1:8080
    export MAIDAN_TOKEN=maid_...        # omit only against an auth-disabled dev server
    PYTHONPATH=sdk/python/src python3 examples/lease_demo/lease_demo.py

Requires Node on PATH for the TypeScript worker (`examples/lease_demo/claim.mjs`).
"""

import os
import pathlib
import subprocess
import sys

from maidan import Client

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN")
HERE = pathlib.Path(__file__).resolve().parent


def thread_id(claim):
    """`claim_next_thread` returns a Thread object or None."""
    return claim.get("id") if isinstance(claim, dict) else None


def main() -> int:
    c = Client(BASE, TOKEN)

    # --- setup: a workspace, two agent members, a channel, two open tasks ---
    ws = c.workspaces.create("lease-demo")["id"]
    # Member creation is a bootstrap op, not a first-class SDK method — use the raw
    # transport. (An orchestrator normally seeds members out of band.)
    planner = c._req("POST", f"/workspaces/{ws}/members", {"handle": "planner", "kind": "agent"})["id"]
    reviewer = c._req("POST", f"/workspaces/{ws}/members", {"handle": "reviewer", "kind": "agent"})["id"]
    channel = c.channels.create(ws, "coordination")["id"]
    c.threads.create(channel, "task-1: audit the login flow")
    c.threads.create(channel, "task-2: benchmark the search path")

    # --- the race: Python worker A and TypeScript worker B claim the same queue ---
    claim_a = thread_id(c.claim_next_thread(channel, {"member_id": planner, "lease_secs": 120}))
    print(f"[python worker]     claimed thread: {claim_a}")

    proc = subprocess.run(
        ["node", str(HERE / "claim.mjs")],
        env={**os.environ, "MAIDAN_URL": BASE, "MAIDAN_CHANNEL": channel, "MAIDAN_MEMBER": reviewer},
        capture_output=True,
        text=True,
    )
    sys.stdout.write(proc.stdout)
    if proc.returncode != 0:
        sys.stderr.write(proc.stderr)
        raise SystemExit("typescript worker failed")
    claim_b = proc.stdout.strip().splitlines()[-1].removeprefix("CLAIM=") or None
    if claim_b == "null":
        claim_b = None

    # --- the queue is now drained: a third claim gets nothing ---
    claim_c = thread_id(c.claim_next_thread(channel, {"member_id": planner, "lease_secs": 120}))
    print(f"[python worker]     third claim (queue drained): {claim_c}")

    # --- the falsifiable assertions ---
    assert claim_a, "python worker should claim an open task"
    assert claim_b, "typescript worker should claim an open task"
    assert claim_a != claim_b, (
        f"DOUBLE-CLAIM: python and typescript both got {claim_a} — the lease board failed"
    )
    assert claim_c is None, f"third claim must be empty (queue drained), got {claim_c}"

    print(
        "\nOK — two tasks, two workers in two languages, two distinct claims, "
        "and the drained queue returns null. Exactly one waiter per task, no double-claim."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
