"""A deploy agent that will not deploy until a person says yes.

It claims a deploy task, opens an approval gate on that thread, and waits. The
gate is durable and silence is never consent: nothing deploys until someone
other than this agent accepts, in `/ui` or with `approve.py`. A decline or a
cancel ends the task without deploying. While the gate is open, no other agent
can claim the thread; this agent's lease is kept alive around the wait.

`DEPLOY_COMMAND` is the deploy itself, run with the task in `$MAIDAN_TASK`.
With none set, a stand-in prints what it would have done.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time

from maidan_http import Claim, Maidan, claim_next, from_creds

LEASE_SECS = int(os.environ.get("LEASE_SECS", "120"))
POLL_SECS = float(os.environ.get("POLL_SECS", "3"))


def wait_for_answer(maidan: Maidan, gate_id: str) -> dict:
    while True:
        gate = maidan.tool("get_approval_gate", {"gate_id": gate_id})
        if gate["state"] != "pending":
            return gate
        time.sleep(POLL_SECS)


def deploy(task: str) -> tuple[int, str]:
    command = os.environ.get("DEPLOY_COMMAND", "").strip()
    if not command:
        return 0, f"stand-in: would deploy now ({task}); set DEPLOY_COMMAND to run one"
    done = subprocess.run(
        command,
        shell=True,
        env={**os.environ, "MAIDAN_TASK": task},
        capture_output=True,
        text=True,
    )
    return done.returncode, "\n".join((done.stdout + done.stderr).strip().splitlines()[-20:])


def work_one(maidan: Maidan, claim: Claim) -> None:
    with claim:
        task = claim.task()
        opened = maidan.tool(
            "request_approval",
            {"prompt": f"Approve: {task}", "thread_id": claim.thread_id},
        )
        gate_id = opened["gate_id"]
        claim.post(f"Waiting for approval (gate {gate_id}) before: {task}")
        print(f"deploy-agent: {claim.thread_id} waiting on gate {gate_id}", flush=True)

        gate = wait_for_answer(maidan, gate_id)
        if gate["state"] != "accepted":
            claim.post(f"Not deploying: the gate was {gate['state']}.")
            claim.finish({"status": "not_deployed", "gate": gate["state"]})
            print(f"deploy-agent: gate {gate['state']}, not deploying", flush=True)
            return

        code, log = deploy(task)
        status = "deployed" if code == 0 else "failed"
        claim.post(f"{status} (exit {code})\n\n{log}")
        claim.finish({"status": status, "exit_code": code, "gate": "accepted"})
        print(f"deploy-agent: {status}", flush=True)


def main() -> int:
    maidan, creds = from_creds()
    once = "--once" in sys.argv
    while True:
        claim = claim_next(maidan, creds["channel_id"], LEASE_SECS)
        if claim is None:
            if once:
                return 0
            time.sleep(POLL_SECS)
            continue
        work_one(maidan, claim)
        if once:
            return 0


if __name__ == "__main__":
    raise SystemExit(main())
