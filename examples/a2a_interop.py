"""A2A v1.0 interop / conformance check against a running Maidan.

Validates the Agent Card shape (§4.4.1) and exercises the JSON-RPC and REST
bindings with the spec's canonical operation names — a lightweight, dependency-
light conformance client (httpx only, no A2A SDK) that doubles as an example.

Run it against the quickstart (auth disabled):

    docker compose -f compose.quickstart.yaml up -d --build   # Maidan on :8080
    pip install "httpx>=0.27"
    python examples/a2a_interop.py                             # exits non-zero on failure

Set MAIDAN_URL / MAIDAN_TOKEN to target an auth-enabled deployment.
"""

from __future__ import annotations

import os
import sys
import uuid

import httpx

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN")
HEADERS = {"content-type": "application/json"}
if TOKEN:
    HEADERS["authorization"] = f"Bearer {TOKEN}"

_failures: list[str] = []


def check(cond: bool, msg: str) -> None:
    status = "ok  " if cond else "FAIL"
    print(f"  [{status}] {msg}")
    if not cond:
        _failures.append(msg)


def rpc(client: httpx.Client, method: str, params: dict | None = None) -> dict:
    body = {"jsonrpc": "2.0", "id": 1, "method": method}
    if params is not None:
        body["params"] = params
    return client.post(f"{BASE}/a2a/v1/rpc", headers=HEADERS, json=body).json()


def main() -> int:
    with httpx.Client(timeout=10.0) as client:
        # 1) Agent Card conformance (§4.4.1).
        print("Agent Card (/.well-known/agent-card.json):")
        card = client.get(f"{BASE}/.well-known/agent-card.json").json()
        check(bool(card.get("name")), "has name")
        check(bool(card.get("description")), "has description")
        check(bool(card.get("version")), "has version")
        ifaces = card.get("supportedInterfaces") or []
        check(len(ifaces) >= 1, "has >=1 supportedInterfaces")
        bindings = {i.get("protocolBinding") for i in ifaces}
        check("JSONRPC" in bindings, "advertises a JSONRPC interface")
        check(
            all(i.get("protocolVersion") for i in ifaces),
            "every interface has a protocolVersion",
        )
        caps = card.get("capabilities") or {}
        check(isinstance(caps, dict), "capabilities is an object")
        check(isinstance(card.get("skills"), list) and card["skills"], "has skills")

        # 2) Seed a workspace/member/channel/thread to target.
        print("Seeding a workspace/thread:")
        ws = client.post(f"{BASE}/workspaces", headers=HEADERS, json={"name": "a2a-interop"}).json()
        wid = ws["id"]
        member = client.post(
            f"{BASE}/workspaces/{wid}/members",
            headers=HEADERS,
            json={"handle": f"agent-{uuid.uuid4().hex[:6]}", "kind": "agent"},
        ).json()
        ch = client.post(
            f"{BASE}/workspaces/{wid}/channels", headers=HEADERS, json={"name": "general"}
        ).json()
        th = client.post(
            f"{BASE}/channels/{ch['id']}/threads", headers=HEADERS, json={"title": "interop"}
        ).json()
        check(bool(th.get("id")), "created a thread")

        # 3) JSON-RPC binding: SendMessage → GetTask (canonical method names, §5.3).
        print("JSON-RPC binding:")
        sent = rpc(
            client,
            "SendMessage",
            {
                "message": {"role": "user", "parts": [{"type": "text", "text": "hi from a2a"}]},
                "metadata": {"maidan": {"threadId": th["id"], "authorId": member["id"]}},
            },
        )
        task = sent.get("result", {}).get("task", {})
        task_id = task.get("id")
        check(bool(task_id), "SendMessage returned a task id")
        check(
            str(task.get("status", {}).get("state", "")).startswith("TASK_STATE_"),
            "task state is a TASK_STATE_* enum value",
        )
        got = rpc(client, "GetTask", {"id": task_id})
        check(got.get("result", {}).get("id") == task_id, "GetTask round-trips the task")
        listed = rpc(client, "ListTasks", {})
        check("tasks" in listed.get("result", {}), "ListTasks returns a tasks array")
        # An unknown method is a proper JSON-RPC method-not-found.
        bad = rpc(client, "NoSuchMethod", {})
        check(bad.get("error", {}).get("code") == -32601, "unknown method → -32601")

        # 4) REST binding (§11): the same task over GET /a2a/v1/tasks/{id}.
        print("REST binding:")
        rest_task = client.get(f"{BASE}/a2a/v1/tasks/{task_id}", headers=HEADERS)
        check(rest_task.status_code == 200, "GET /a2a/v1/tasks/{id} → 200")
        check(rest_task.json().get("id") == task_id, "REST GetTask round-trips the task")
        rest_card = client.get(f"{BASE}/a2a/v1/extendedAgentCard", headers=HEADERS)
        check(rest_card.status_code == 200, "GET /a2a/v1/extendedAgentCard → 200")

    print()
    if _failures:
        print(f"A2A interop: {len(_failures)} FAILED")
        return 1
    print("A2A interop: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
