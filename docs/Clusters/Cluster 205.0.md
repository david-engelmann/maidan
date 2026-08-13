# Cluster 205.0 — correctness: transactional outbox (foundation)

**Theme:** Program A (security & correctness round 2), part 4 — begin the
transactional-outbox refactor (the twice-deferred atomic dual-write). This
cluster lands the **pattern** + the first two mutations; the rest migrate in
follow-up clusters (the user chose the full multi-cluster path).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v205.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `events::append_in_tx(&mut tx, event)` — append event + outbox on a caller-supplied tx (both backends) | `store/{postgres,sqlite}/events.rs` |
| `create_channel_with_event` / `create_thread_with_event` — insert + append in one tx | `store/{postgres,sqlite}/{channels,threads}.rs`, `store.rs`, `*/mod.rs` |
| `publish_stored` — post-commit best-effort bus notify | `routes/mod.rs` |
| Wire channel + thread create routes | `routes/{channel,thread}.rs` |

## Why

A mutation committed its domain row in one transaction, then `publish()` appended
the durable `Event` in a **separate** transaction. A crash (or a permanent append
failure) between the two committed the domain state but lost the event forever —
no notification, no at-least-once delivery, no search reindex, no `/ui` live
frame. The domain and the event stream silently diverge. Cluster 184 hardened this
(retry + `maidan_event_append_failures_total`) but couldn't make it atomic. This
is the last open item on the correctness backbone.

## The change

`append_event` already wrote the event + its outbox row atomically (one tx). The
missing piece was making the **domain mutation** share that tx. So:

1. Extract `append_in_tx(&mut tx, event) -> StoredEvent` from `append` (both
   backends). `append` is now `begin → append_in_tx → commit`.
2. A `*_with_event` store method opens a tx, does the domain insert on it,
   resolves the event's context **in the same tx** (e.g. `SELECT workspace_id
   FROM maidan_channels WHERE id = ?` for `ThreadCreated`), builds the `Event`,
   calls `append_in_tx`, and commits — returning `(DomainResult, StoredEvent)`.
   The row and the event commit **atomically or not at all**.
3. The route calls the `*_with_event` variant, then `publish_stored(state,
   stored)` — the *best-effort* live notification, run **after** the durable
   commit, hydrating the event from `stored.payload`. A bus/relay hiccup can no
   longer undo a committed mutation (the old `publish` did the durable append
   *and* the notify; `publish_stored` does only the notify).

Behaviour is unchanged — the same events reach the stream; only the
crash-consistency guarantee is new. This cluster applies it to the two simplest
mutations (channel + thread create); the rest follow.

## Exit criteria

- The channel/thread row and their creation events commit atomically; the pattern
  (`append_in_tx` + `*_with_event` + `publish_stored`) is established and
  reviewable — **met**.
- `v205.0.0` tagged.

## Verification & limits

- `event_log::create_with_event_commits_row_and_event` (store, SQLite): after one
  `create_*_with_event` call, both the domain row and the durable event exist, and
  the returned event *is* the logged one.
- Behaviour-preserving: `event_emission_e2e`, `ws_subscribe_e2e`, `mcp_e2e`,
  `channel_access_e2e`, `thread_transition_e2e` + the full store suite (both
  backends) green — the same `ChannelCreated`/`ThreadCreated` events still reach
  subscribers.
- **Limit (accepted, tracked):** this is a **multi-cluster migration**. Only
  channel/thread create are atomic so far; every other mutation still uses the
  retry-hardened `publish()` (a *temporary mixed-atomicity* the user accepted by
  choosing the full-refactor path). Follow-ups migrate social (reactions / votes /
  pins / mentions), thread transitions / assignments, and — hardest — the
  slash-edit-**entangled** message-post path, where the event must reflect the
  *post-slash-edit* message (so it's built in-store after the edit). When all
  mutations are migrated, the non-atomic `publish()` is removed.

## References

- [[Retros/Cluster 205.0]]; `store/*/events.rs`, `routes/mod.rs`. Program:
  [[Roadmap]] + [[Open Work]] + memory `maidan-next-arc-program` (Program A).
  Completes the Cluster 184 deferral's foundation.
