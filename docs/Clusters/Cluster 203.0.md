# Cluster 203.0 — security: DM/group-DM participation (subscribe + metadata)

**Theme:** Program A (security & correctness round 2), part 2 — close the
DM/group-DM participation gaps that Cluster 180 left on the real-time subscribe
path and the metadata read routes (the events/metadata analog of the 180 read
gap).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v203.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Subscribe gate: `expand_event_filter` runs `ensure_thread_access` on the resolved `thread_id` | `dm.rs` (+ `ws.rs` reorder, `mcp_stream.rs` pass auth) |
| Metadata reads gated: `get_dm`/`get_group_dm` (session-participant), `list_*` (session self-only) | `dm.rs`, `group_dm.rs` |
| Auth-enabled subscribe-gate e2e | `tests/dm_participation_e2e.rs` |

## Why

Cluster 180 made `ensure_thread_access` DM-participant-aware and applied it on the
generic thread/message *routes* — but two DM surfaces were missed:

- **Subscribe (leaks DM *content*).** `expand_event_filter` took a caller-supplied
  `dm_conversation_id`, fetched the DM with **no participant check**, and set
  `filter.thread_id`. It runs before `apply_subscribe_grants`, which exempts the
  shared `__dm__` channel — so anyone with `event:subscribe` could tail any
  DM/group-DM's live messages on `GET /mcp/stream` (or WS), by the
  `dm_conversation_id` *or* by supplying the `__dm__` `thread_id` directly.
- **Metadata reads (leak the roster).** `GET /dm/:id` and `/group-dms/:id`
  returned a conversation's participants + thread to any workspace member, and
  `list` enumerated any member's DM graph.

## The fix

**Subscribe:** `expand_event_filter` now takes `auth` and, after resolving
`thread_id` (from `dm_conversation_id` or a direct filter), runs
`maidan_auth::ensure_thread_access` (DM-participant-aware, Cluster 180;
bypass-exempt) on it. This closes **both** entry paths and both transports (WS +
MCP-SSE both call it). `ws.rs` resolves the caller's `ctx` *before* the expand so
the check has an identity.

**Metadata:** a **session** caller must be a participant to `get`, and may only
`list` its own conversations (via the Cluster 202 `ensure_acting_member` rule).

**The model split (deliberate):** metadata reads use the **session-only** guard —
a **bearer** is the orchestrator model and legitimately reads/lists on behalf of
any member (a bot token manages members' DMs; `group_dm_e2e` mints exactly such a
token). But **message content** (the subscribe stream, and the thread GET from
Cluster 180) requires participation even for a bearer — reading a DM's messages is
higher-sensitivity than reading its roster. So: roster/list → session-guarded;
content → participant-required.

## Exit criteria

- A non-participant cannot tail a DM via `dm_conversation_id` or `thread_id`; a
  session cannot read/enumerate others' DM metadata — **met**.
- `v203.0.0` tagged.

## Verification & limits

- `dm_participation_e2e::non_participant_cannot_tail_a_dm` (auth enabled): a
  non-participant bearer is `403` on `GET /mcp/stream` via **both**
  `dm_conversation_id` and the DM's `thread_id`; a participant is `200`.
- Regression: `dm_e2e`, `group_dm_e2e`, `ws_subscribe_e2e`, `subscribe_grants_e2e`,
  `mcp_stream_at_least_once_e2e`, `ui_collab_e2e`, `presence_ws_e2e` all green —
  the group-DM list test (a bot listing a member's DMs) drove the session-only
  vs bearer-orchestrator model decision.
- Limit: the metadata `get`/`list` guards are session-only; a **bearer** may still
  read any conversation's roster (orchestrator model — intentional, same as the
  202 write model). Whether a workspace bearer should be roster-restricted is a
  policy question, documented.

## References

- [[Retros/Cluster 203.0]]; `dm.rs`, `ws.rs`, `mcp_stream.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Program A). Completes the
  DM-participation surface Cluster 180 ([[Retros/Cluster 180.0]]) opened.
