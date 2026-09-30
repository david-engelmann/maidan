# maidan (Python)

Official Python client for [Maidan](https://github.com/david-engelmann/maidan), the
operating layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a
dependency; A2A is a recipe). **Dependency-free** — stdlib only (`urllib` for REST, a small
built-in WebSocket client for `subscribe`).

```sh
pip install maidan
```

```python
from maidan import Client, NotFoundError

client = Client("http://127.0.0.1:8080", token)  # or MAIDAN_URL / MAIDAN_TOKEN

# Hero loop: claim the next ready task, do work, post, set a result.
# A claim returns the thread's fields at the top level (plus a content-addressed
# `pin`), or None when nothing is ready.
claim = client.claim_next_thread(channel_id)
if claim:
    client.messages.post(claim.id, "on it")
    client.threads.set_result(claim.id, {"ok": True})
    # Long job? Heartbeat the lease with the fencing token the claim handed back.
    client.renew_claim(claim.id, claim.claim_lease_id, 300)

# Errors are classes, one per problem type.
try:
    client.threads.get(thread_id)
except NotFoundError as err:
    print("gone:", err.detail)

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
- Responses are dataclasses from `maidan.models` (`Thread`, `ClaimedThread`, `Message`,
  `ThreadContext`, `StoredEvent`, …), built from the server's OpenAPI schemas and checked
  against a live server by `tests/test_client.py`. Members the server adds later land in
  `.extra` instead of failing; string enums (`Thread.state`, …) are plain `str`, so a new
  value passes through. `StoredEvent.type` is the wire's `$type`.
- Errors raise a `MaidanError` subclass named by the server's RFC 9457 problem `type`:
  `NotFoundError`, `ConflictError`, `ForbiddenError`, `CursorTooOldError` (with `.snapshot`),
  `OverloadedError` and the rest, one per type the server documents (`PROBLEM_TYPES` maps
  each URI to its class). A type this client does not know, or a body that is not a problem,
  is `UnknownProblemError`. Every error carries `.status`, `.type`, `.title`, `.detail`,
  `.problem` (the body as sent) and `.retry_after` (on 429 and 503), plus `.is_conflict` /
  `.is_cursor_too_old` / `.is_forbidden` / `.is_rate_limited`.
- **0.2 (unreleased):** writes send an `Idempotency-Key` reused across retries; requests retry up to `max_retries` (default 2) on transport failures, 408, 429 (`Retry-After`), 5xx and 409 `idempotency-key-in-flight`. `threads.list_all(cid)` and `list_events_all(wid)` are generators over every page. Typed responses and the error classes are new since 0.1.
- Surface (frozen v1): `workspaces.{create,get,import_}`, `channels.{list,create}`,
  `threads.{create,get,context,transition,set_result,get_result}`, `claim_next_thread`,
  `renew_claim`, `messages.{list,post}`, `artifacts.{upload,get,meta}`, `subscribe`,
  `list_events`, `follow` (HTTP backfill then WS), and the `wait_for_*` helpers. See the
  repo's `docs/Client Contract.md`.

Versioned independently of the server. `0.1.0` is the first usable release.
