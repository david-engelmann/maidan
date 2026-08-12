# Cluster 196.0 retro — an agent can await its next mention now

> Tag **`v196.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 7.

## What shipped

- `wait_for_mention` — a blocking MCP tool that subscribes to the event bus for
  the member's `MentionRecorded` events and returns the next one (or `null` after
  a `timeout_ms` window). RBAC-filtered by `can_access_thread`; requires
  `workspace:read`.

## Surprises / decisions

- **The bus already had exactly the primitive I needed.** `EventBus::subscribe`
  takes an `EventFilter` and returns a pre-filtered stream, and
  `EventFilter { member_id, kinds: {MentionRecorded} }` matches a mention of that
  member exactly (`event.member_id()` returns the *mentioned* member for
  `MentionRecorded`). So the handler is a subscribe + a `timeout_at(stream.next())`
  loop — no new store method, no new bus surface. The MCP crate just needed
  `futures` for `StreamExt::next` (its first stream consumer).
- **Live-only is the honest scope, and it composes.** A tempting richer design is
  "return any mention since `after_id`, else block" — but that duplicates the SSE
  transport's replay logic in a tool. I kept `wait_for_mention` live-only and
  documented drain-then-wait (`get_inbox` then `wait_for_mention`); the resumable
  `GET /mcp/stream` is the at-least-once path. The two are complementary, not
  redundant. `after_id` catch-up is a clean future add if agents need it.
- **RBAC on a primitive that returns *nothing but ids*.** The `MentionRecorded`
  event carries no message body — just ids. It would have been defensible to
  return it unfiltered (a mention is addressed to you). I filtered anyway: even
  the *timing/existence* of a mention in a private channel you're not in is a
  signal, and the RBAC arc's rule is "no cross-channel leakage". Skipping and
  continuing (not erroring) keeps the semantics clean: you get your *accessible*
  next mention, or a timeout.
- **Testing a blocking primitive without a sleep-then-signal.** The subscribe
  must happen before the publish, or the live-only stream misses it.
  `tokio::join!(call_tool(...), async { sleep; publish })` polls the waiter first
  — it subscribes and parks on the stream within its first poll — then the
  delayed publisher fires. Deterministic, and it sidesteps the repo's standing
  ban on `Notify::notify_waiters` for producer/poller signaling.

## Decisions

- **`workspace:read`**, matching the other member-scoped reads (`list_mentions`,
  `get_inbox`) — awaiting a mention is a read, not a write.
- **No `enforce_channel_access` arm.** Like `list_assigned_threads`, this is a
  member-scoped tool the pre-dispatch channel gate can't cover (the arg is a
  `member_id`, not a channel/thread), so the handler does its own per-result
  `can_access_thread` filter.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `wait_for_mention` — blocking long-poll for the member's next accessible mention | `maidan-mcp` (`member.rs` + `mod.rs` + `catalog.rs` + contracts) |

## Risks identified + still open

- **A slow/blocked tool call holds a request for up to `timeout_ms`** (≤ 300 s).
  That's inherent to a long-poll; the clamp bounds it, and each waiter is one
  cheap bus subscription. Open: **live-only** (a mention between drain and wait is
  not returned by that call — it stays in the inbox); `after_id` catch-up is the
  future close.

## Forward look

Arc C's last item: structured tool-call transcripts (typed `tool_use`/
`tool_result` content threaded through a conversation). Then Arc D
(performance & scale) — load harness first.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes the mention
loop opened by [[Retros/Cluster 149.0]] + [[Retros/Cluster 150.0]].
