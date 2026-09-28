"""A2A v1.0 walkthrough against a running Maidan, checking each answer.

A dependency-light client (httpx only, no A2A SDK) that shows the protocol as
Maidan speaks it: the Agent Card, a conversation over the JSON-RPC binding
(a context is a Maidan thread), the same task over the HTTP+JSON binding, a
streamed send, and version negotiation. The official conformance suite is
`scripts/a2a-tck.sh`; this is the readable example.

    docker compose -f compose.quickstart.yaml up -d --build --wait   # Maidan on :8080
    docker compose -f compose.quickstart.yaml exec maidan maidan init --workspace demo
    pip install "httpx>=0.27"
    export MAIDAN_TOKEN=maid_...                       # from `maidan init`
    python examples/a2a_interop.py                     # exits non-zero on failure

MAIDAN_URL targets a deployment other than localhost:8080.
"""

from __future__ import annotations

import json
import os
import sys
import uuid

import httpx

BASE = os.environ.get("MAIDAN_URL", "http://127.0.0.1:8080")
TOKEN = os.environ.get("MAIDAN_TOKEN")
# Every A2A request names the protocol version it speaks (§3.6.2).
HEADERS = {"A2A-Version": "1.0", "authorization": f"Bearer {TOKEN}"}

_failures: list[str] = []


def check(cond: bool, msg: str) -> None:
    print(f"  [{'ok  ' if cond else 'FAIL'}] {msg}")
    if not cond:
        _failures.append(msg)


def rpc(client: httpx.Client, method: str, params: dict | None = None) -> dict:
    body = {"jsonrpc": "2.0", "id": str(uuid.uuid4()), "method": method, "params": params or {}}
    return client.post(f"{BASE}/a2a/v1/rpc", headers=HEADERS, json=body).json()


def message(text: str, context_id: str | None = None) -> dict:
    msg = {"messageId": str(uuid.uuid4()), "role": "ROLE_USER", "parts": [{"text": text}]}
    if context_id:
        msg["contextId"] = context_id
    return {"message": msg}


def main() -> int:
    if not TOKEN:
        print("set MAIDAN_TOKEN to a token from `maidan init`", file=sys.stderr)
        return 2
    with httpx.Client(timeout=10.0) as client:
        print("Agent Card:")
        card = client.get(f"{BASE}/.well-known/agent-card.json")
        check(card.status_code == 200 and "etag" in card.headers, "served with an ETag")
        card = card.json()
        bindings = {i["protocolBinding"] for i in card.get("supportedInterfaces", [])}
        check({"JSONRPC", "HTTP+JSON"} <= bindings, "advertises JSONRPC and HTTP+JSON")
        check("bearer" in card.get("securitySchemes", {}), "declares bearer auth")

        print("JSON-RPC: a conversation")
        first = rpc(client, "SendMessage", message("hello from an A2A client")).get("result", {})
        task = first.get("task", {})
        context = task.get("contextId")
        check(task.get("status", {}).get("state") == "TASK_STATE_COMPLETED", "delivered")
        check(bool(context), "a new context (a new Maidan thread)")
        reply = rpc(client, "SendMessage", message("a follow-up", context)).get("result", {})
        check(reply.get("task", {}).get("contextId") == context, "the follow-up joins it")
        got = rpc(client, "GetTask", {"id": task.get("id"), "historyLength": 1}).get("result", {})
        history = got.get("history", [])
        check(
            history and history[0]["parts"][0]["text"] == "hello from an A2A client",
            "GetTask renders the message as history",
        )
        listed = rpc(client, "ListTasks", {"contextId": context}).get("result", {})
        check(len(listed.get("tasks", [])) == 2, "ListTasks finds both tasks in the context")
        missing = rpc(client, "GetTask", {"id": str(uuid.uuid4())})
        check(missing.get("error", {}).get("code") == -32001, "unknown task: TaskNotFound")

        print("HTTP+JSON: the same task")
        rest = client.get(f"{BASE}/a2a/v1/tasks/{task.get('id')}", headers=HEADERS)
        check(rest.status_code == 200 and rest.json().get("id") == task.get("id"), "GET /tasks/{id}")
        with client.stream(
            "POST", f"{BASE}/a2a/v1/message:stream", headers=HEADERS, json=message("streamed", context)
        ) as stream:
            events = [
                json.loads(line[5:]) for line in stream.iter_lines() if line.startswith("data:")
            ]
        check(
            [next(iter(e)) for e in events] == ["task", "statusUpdate"],
            "message:stream sends the task, then its completion",
        )

        print("Version negotiation:")
        old = client.get(
            f"{BASE}/a2a/v1/tasks/{task.get('id')}",
            headers={**HEADERS, "A2A-Version": "0.3"},
        )
        check(
            old.status_code == 400
            and old.json()["error"]["details"][0]["reason"] == "VERSION_NOT_SUPPORTED",
            "A2A-Version 0.3 is refused",
        )

    print()
    if _failures:
        print(f"A2A walkthrough: {len(_failures)} FAILED")
        return 1
    print("A2A walkthrough: all checks passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
