# Cluster 196.0 — agentic: `wait_for_mention` (blocking MCP long-poll)

**Theme:** Arc C (agentic task-queue depth), part 7 — give an MCP-native agent a
way to *await* its next @mention instead of polling `get_inbox`.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v196.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `wait_for_mention` handler — subscribe to the bus, block for the next `MentionRecorded`, RBAC-filter, timeout to `null` | `maidan-mcp/src/tools/member.rs` |
| Capability arm (`workspace:read`) + dispatch arm | `maidan-mcp/src/tools/mod.rs` |
| Catalog schema (`member_id` required, `timeout_ms` optional) | `maidan-mcp/src/tools/catalog.rs` |
| Contract entries (sorted) | `contracts/mcp-tool-names.json`, `contracts/mcp-capability-map.json` |
| `futures` dep for `StreamExt::next` | `maidan-mcp/Cargo.toml` |

## Why

Cluster 149 gave an MCP agent `list_mentions`/`get_inbox` (discover it was
@mentioned) and Cluster 150 gave `GET /mcp/stream` mention filters (an SSE
"await my mention" stream). But an agent that can't hold a long-lived SSE
connection — a request/response tool loop — had only *polling*: call `get_inbox`,
sleep, call again. `wait_for_mention` is the synchronous, ergonomic primitive:
one tool call that blocks until the agent is next mentioned. It closes the loop
opened by 149 (discover) + 150 (await over SSE) with an await-over-tool-call.

## The design

The handler subscribes to the event bus with
`EventFilter { member_id: Some(caller), kinds: {MentionRecorded} }` — the bus
pre-filters, so the stream yields exactly this member's mentions — and awaits the
next envelope with `tokio::time::timeout_at(deadline, stream.next())`. On a match
it returns the mention event; on the deadline it returns `null`. A
`BusItem::Lagged` marker (buffer overflow) is skipped rather than treated as a
timeout, still bounded by the same deadline.

**RBAC.** A mention is addressed *to* the member, but it lives in a thread. If the
caller can't `can_access_thread` that thread (e.g. a private channel it isn't a
member of), the handler skips it and keeps waiting — so the tool never reveals
even the *existence/timing* of activity in a thread the caller couldn't otherwise
see. Bypass callers skip the filter (parity with `list_assigned_threads`).

## Exit criteria

- A mention published after the call returns from `wait_for_mention`; a private
  mention the caller can't access is filtered → the call times out to `null` —
  **met**.
- `v196.0.0` tagged.

## Verification & limits

- `wait_for_mention_returns_next_mention_and_filters_private` (maidan-mcp): a
  `tokio::join!` of the waiter and a delayed `publish_event` — the public mention
  wakes the waiter (asserts kind/member/thread), a private-channel mention is
  filtered so the waiter times out to `null`. `join!` polls the waiter first (it
  subscribes, then parks on the stream) then the delayed publisher, so there's no
  subscribe race and no sleep-then-signal.
- `mcp_capability_matrix_e2e` (both legs) + the catalog/capability contract-sync
  tests stay green with the new tool.
- Limit: **live-only.** `wait_for_mention` sees mentions recorded *after* it
  subscribes — a mention that arrived between the agent's last `get_inbox` and
  this call is not returned by *this* call (it's still in the inbox). The
  documented pattern is drain-then-wait; the resumable `GET /mcp/stream` SSE
  transport (with `Last-Event-ID`) is the at-least-once alternative when a missed
  mention is unacceptable. A future `after_id` catch-up arg could close the gap.

## References

- [[Retros/Cluster 196.0]]; `maidan-mcp/src/tools/member.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Arc C). Completes the
  discover ([[Retros/Cluster 149.0]]) + await-over-SSE ([[Retros/Cluster 150.0]])
  mention loop.
