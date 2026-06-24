# Cluster 129.0 retro — Hardening: error-visibility + bounded buffers

> Tag **`v129.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. Third
> cluster of the hardening sweep (127 reconcile → 128 A2A → **129** → 130).

## What shipped

- **Bounded MCP streamable buffer** (`streamable_session.rs`): the per-session
  SSE channel was `unbounded_channel()` — a slow/stalled client grew server
  memory without limit. Now `channel(256)` + non-blocking `try_send`; a full
  buffer logs and fails the push (the caller already treats that as a gone
  session). Test: filling the buffer fails the next push without blocking.
- **Outbox quarantine error surfaced** (`outbox_relay.rs`): a failed
  `quarantine()` was `let _ = …`, leaving the row pending → it would retry
  forever. Now logged; the next tick retries the quarantine.
- **`unreachable!()` → typed errors** in three live request handlers
  (`delivery_ops` get/replay, `mcp/resources` read).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Deferred | Supervise the Prometheus upkeep `std::thread` | Process-lifetime daemon; supervising adds churn without real benefit. |
| 130 | MCP + observability test-coverage uplift | The thin-coverage finding is its own cluster. |

## Surprises

- **The lock forced the channel design.** `push` holds the sessions mutex, so a
  bounded channel could not `await` on capacity (it would stall every session).
  `try_send` + fail-on-full was the only correct non-blocking choice — and it
  happens to match the existing `push`→false "gone session" contract exactly, so
  callers needed no change.
- **The `unreachable!()`s were genuinely unreachable** (the kind is validated
  upstream) — but they sat in *live request handlers*, where a future change to
  the validator would turn a bad input into a process panic. Converting them to
  typed errors is cheap insurance.

## Decisions

- **Drop-on-full, not grow-on-full.** A slow MCP client is disconnected and must
  reconnect + re-sync, rather than consuming unbounded memory — the right
  backpressure for a notification stream.
- **Surface, don't swallow.** The quarantine error is logged and left to retry,
  not silently dropped; visibility over false safety.
- **Defer the daemon-thread cosmetics.** Not every scan finding is worth acting
  on; the unsupervised upkeep thread is harmless at process lifetime.

## Capability table extension

| Capability | Where |
|------------|-------|
| Bounded MCP streamable session buffer (no memory-exhaustion) | `crates/maidan-mcp/src/streamable_session.rs` |
| Outbox quarantine-failure visibility | `crates/maidan-server/src/outbox_relay.rs` |

## Risks identified + still open

- **Drop-on-full loses notifications for a slow client.** Acceptable (client
  reconnects + re-syncs), and now logged so it's observable.

## Forward look

Last core cluster of the sweep: **Cluster 130** — test-coverage uplift for the
thinnest areas (MCP crate, observability env-parsing). Then the two optional
clusters (unify delivery tables, global admin audit API) pending confirmation.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
