# Cluster 295.0 retro — Python SDK (0.1.0)

> Tag **`v295.0.0`**. Phase XXIV (post-gate hardening). SDK arc, part 2. No new gate tag.

## What shipped

The second usable language client, to the frozen v1 contract ([[Client Contract]]),
verified black-box through the Cluster-294 harness:

- **`sdk/python/src/maidan/client.py`** — a `Client` (REST + WebSocket), **dependency-free
  (stdlib only)**: REST rides `urllib`, and `subscribe` rides a small hand-rolled RFC-6455
  client (`_WebSocketConn`) — handshake, one masked send, a receive loop for text frames
  (auto-pong on ping, close handling, fragmentation reassembly). Namespaced surface
  (`workspaces`/`channels`/`threads`/`messages`/`artifacts`, snake_case per the contract),
  `claim_next_thread` / `renew_claim`, `subscribe(filter, on_event, on_error=None)`, and the
  `wait_for_{result,mention,ready}` helpers. `client.mcp_url` is `{base}/mcp/streamable`.
- **`MaidanError`** — `.status` + parsed `.body`, `.is_conflict` / `.is_forbidden` /
  `.is_rate_limited`, `.retry_after` (from `Retry-After` on a 429).
- **`sdk/python/tests/test_client.py`** — a `pytest` black-box suite (hero loop; `get_result`
  404 on an unset thread; claim-next; error surfacing; WS subscribe). **5/5 pass** locally.
- **`pyproject.toml`** bumped 0.0.1 → **0.1.0** (homepage → the docs site, `dependencies = []`,
  a `test` extra, hatch wheel packages); **`README.md`** rewritten with real usage.
- **`scripts/sdk-test.sh`** Python arm runs the suite with `PYTHONPATH=src` (no install step).

Verified locally: **5/5 tests pass** against a source-built server.

## Surprises / decisions

- **Dependency-free WebSocket, hand-rolled.** Python's stdlib has `urllib` (REST) but **no
  WebSocket client**. Rather than take a third-party dep (`websocket-client`) or farm the WS
  out to an optional extra, `subscribe` ships a ~120-line stdlib RFC-6455 client. It's small
  because Maidan's use is narrow (connect → one text send → read small JSON text frames), and
  it's *verifiable* — the black-box test drives the real handshake + a real `message_posted`
  frame against the live server, so a framing bug fails the test. This keeps `pip install
  maidan` truly zero-dep.
- **snake_case surface** (`threads.set_result`, `claim_next_thread`, `wait_for_result`) — the
  contract lists the canonical names in snake_case; Python keeps them (TS adapted to
  camelCase). `workspaces.import_` takes a trailing underscore (`import` is reserved).
- Same **`set_result` write constraint** as TS (Cluster 294): under the auth-disabled harness
  the acting member is nil (a NOT-NULL `produced_by` FK), so the test exercises the result
  route via `get_result` → 404; the server's `thread_result_e2e` proves the write.

## Capability table extension

New Python client (`sdk/python`, 0.1.0), dependency-free (stdlib REST + a hand-rolled WS).
No server capability change.

## Risks identified + still open

- **Not published to PyPI** — needs a PyPI token as a repo secret + a release trigger on an
  `sdk-*` tag; logged in [[Open Work]].
- The hand-rolled WS handles the frames Maidan emits (small text, ping, close, fragmentation);
  exotic server behavior (per-message compression, huge frames) is out of scope for 0.1.0.

## Forward look

The SDK arc continues: **296 Go → 297 Rust**, each to a usable 0.1.0 against the same frozen
contract + a black-box suite through `scripts/sdk-test.sh`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 294.0]].
Built under the standing SDK-arc authorization (usable 0.1.0, all four languages).
