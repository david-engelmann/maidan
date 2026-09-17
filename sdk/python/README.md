# maidan (Python)

Official Python client for [Maidan](https://github.com/david-engelmann/maidan), the
operating layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a
dependency; A2A is a recipe). **Dependency-free** — stdlib only (`urllib` for REST, a small
built-in WebSocket client for `subscribe`).

```sh
pip install maidan
```

```python
from maidan import Client

client = Client("http://127.0.0.1:8080", token)  # or MAIDAN_URL / MAIDAN_TOKEN

# Hero loop: claim the next ready task, do work, post, set a result.
# A claim returns the thread's fields at the top level (plus a content-addressed
# `pin`), or None when nothing is ready.
claim = client.claim_next_thread(channel_id, {"member_id": member_id})
if claim:
    tid = claim["id"]
    client.messages.post(tid, member_id, "on it")
    client.threads.set_result(tid, {"ok": True})
    # Long job? Heartbeat the lease with the fencing token the claim handed back.
    client.renew_claim(tid, member_id, claim["claim_lease_id"], 300)

# React to work instead of polling.
sub = client.subscribe(
    {"workspace_id": wid, "kinds": ["message_posted"]},
    lambda e: print("event", e["kind"], e.get("thread_id")),
)
# sub.close()

# Or block until a specific signal (wraps subscribe):
ready = client.wait_for_ready(wid)  # event dict or None on timeout
```

- Constructor: `Client(base_url=None, token=None, *, timeout=30.0)` — defaults from
  `MAIDAN_URL` / `MAIDAN_TOKEN`; explicit args win. `client.mcp_url` is
  `{base_url}/mcp/streamable`.
- Errors raise `MaidanError` (`.status`, `.body`, `.retry_after` on 429, `.is_conflict` /
  `.is_cursor_too_old` / `.is_forbidden` / `.is_rate_limited`).
- Surface (frozen v1): `workspaces.{create,get,import_}`, `channels.{list,create}`,
  `threads.{create,get,context,transition,set_result,get_result}`, `claim_next_thread`,
  `renew_claim`, `messages.{list,post}`, `artifacts.{upload,get,meta}`, `subscribe`,
  `list_events`, `follow` (HTTP backfill then WS), and the `wait_for_*` helpers. See the
  repo's `docs/Client Contract.md`.

Versioned independently of the server. `0.1.0` is the first usable release.
