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

import ipaddress
import json
import os
import pathlib
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
import uuid

from maidan import Client

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080").strip().rstrip("/")
TOKEN = os.environ.get("MAIDAN_TOKEN") or ""
WORKSPACE = (os.environ.get("MAIDAN_WORKSPACE") or "").strip()
HERE = pathlib.Path(__file__).resolve().parent
USER_AGENT = "maidan-lease-demo/0.1"


def _loopback(host: str) -> bool:
    host = host.strip().lower()
    if host.startswith("[") and host.endswith("]"):
        host = host[1:-1]
    if host == "localhost":
        return True
    try:
        return ipaddress.ip_address(host).is_loopback
    except ValueError:
        return False


def refuse_cleartext(url: str, token: str) -> None:
    """Do not send a bearer token over cleartext except to this machine."""
    if not token or os.environ.get("MAIDAN_ALLOW_INSECURE_HTTP") == "1":
        return
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "http" or _loopback(parsed.hostname or ""):
        return
    print(
        f"refusing to send MAIDAN_TOKEN over cleartext to {url}; "
        "use https, or set MAIDAN_ALLOW_INSECURE_HTTP=1 for a trusted network",
        file=sys.stderr,
    )
    raise SystemExit(2)


def post(path: str, body: dict, token: str):
    """Call a lifecycle route that is outside the deliberately small SDK v1 surface."""
    headers = {
        "content-type": "application/json",
        "user-agent": USER_AGENT,
        "idempotency-key": str(uuid.uuid4()),
    }
    if token:
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


def workspace_id() -> str:
    """The workspace on the token, from the environment or from GET /me."""
    if WORKSPACE:
        return WORKSPACE
    if not TOKEN:
        print("MAIDAN_WORKSPACE is required when MAIDAN_TOKEN is unset", file=sys.stderr)
        raise SystemExit(2)
    request = urllib.request.Request(
        f"{BASE}/me",
        headers={"authorization": f"Bearer {TOKEN}", "user-agent": USER_AGENT},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            body = json.loads(response.read().decode("utf-8"))
    except urllib.error.HTTPError as error:
        detail = error.read().decode("utf-8", errors="replace")
        print(f"GET /me failed: HTTP {error.code}: {detail}", file=sys.stderr)
        raise SystemExit(2) from error
    except (urllib.error.URLError, OSError, json.JSONDecodeError) as error:
        print(f"GET /me failed: {error}", file=sys.stderr)
        raise SystemExit(2) from error
    found = body.get("workspace_id") if isinstance(body, dict) else None
    if not isinstance(found, str) or not found:
        print("GET /me did not return a workspace_id; set MAIDAN_WORKSPACE", file=sys.stderr)
        raise SystemExit(2)
    return found


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
    refuse_cleartext(BASE, TOKEN)
    workspace = workspace_id()
    admin = Client(BASE, TOKEN)
    run_id = uuid.uuid4().hex[:8]

    # --- setup: two member-bound workers, one channel, two open tasks ---
    planner = admin.members.create(workspace, f"planner-{run_id}").id
    reviewer = admin.members.create(workspace, f"reviewer-{run_id}").id
    worker_caps = ["workspace:read", "thread:transition"]
    planner_token = admin.tokens.mint(workspace, planner, worker_caps).secret
    reviewer_token = admin.tokens.mint(workspace, reviewer, worker_caps).secret
    planner_client = Client(BASE, planner_token)
    channel = admin.channels.create(workspace, f"coordination-{run_id}").id
    admin.threads.create(channel, f"task-1 ({run_id}): audit the login flow")
    admin.threads.create(channel, f"task-2 ({run_id}): benchmark the search path")

    # --- the race: Python worker A and TypeScript worker B claim the same queue ---
    claim_a = planner_client.claim_next_thread(channel, {"lease_secs": 120})
    assert claim_a, "python worker should claim an open task"
    claim_a_id = claim_a.id
    lease_a = claim_a.claim_lease_id
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
                "tokens": {"input": 120, "output": 0, "cache_read": 0, "cache_write_5m": 0, "cache_write_1h": 0},
                "usd_micros": 0,
                "price_snapshot": {
                    "input_usd_micros_per_million": 0,
                    "output_usd_micros_per_million": 0,
                    "cache_read_usd_micros_per_million": 0,
                    "cache_write_5m_usd_micros_per_million": 0,
                    "cache_write_1h_usd_micros_per_million": 0,
                },
                "turns": 1,
            },
            planner_token,
        )
        assert usage.get("stopped") is False, f"unexpected budget stop: {usage}"
        renewed = planner_client.renew_claim(claim_a_id, lease_a, 300)
        assert renewed.assignment_expires_at, "renew must preserve a finite lease"
        print("[python worker]     acknowledged, reported usage, renewed")

        # The TypeScript worker runs the same lifecycle, checks that a third
        # claim is null while both leases are held, then releases its own claim.
        try:
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
                check=True,
                timeout=30,
            )
        except subprocess.CalledProcessError as error:
            sys.stdout.write(error.stdout or "")
            sys.stderr.write(error.stderr or "")
            raise RuntimeError("typescript worker failed") from error
        sys.stdout.write(proc.stdout)
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
        def release() -> None:
            released = post(
                f"/threads/{claim_a_id}/claim/release",
                {"claim_lease_id": lease_a},
                planner_token,
            )
            if released.get("assignee_id") is not None:
                raise RuntimeError("release must return the task to the queue")
            print("[python worker]     released claim on exit")

        if sys.exc_info()[1] is not None:
            try:
                release()
            except Exception as error:
                print(f"warning: release failed: {error}", file=sys.stderr)
        else:
            release()

    print(
        "\nOK — two distinct fenced claims, both workers acknowledged/reported/renewed, "
        "the held queue returned null, and both workers released cleanly."
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
