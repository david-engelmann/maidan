# Cluster 344.0 retro — bounded-concurrency notification fan-out (audit P2)

> Tag **`v344.0.0`**. Phase XXIV (post-gate hardening). **Cluster 13 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The notification router is a **serial** bus consumer: it processes one event before the next. A
`MessagePosted` fanned out to its followers in a sequential loop — `2 × followers` store
round-trips (a mute check + a conditional insert each), one after another — so a widely followed
message head-of-line-blocked the entire notification pipeline while those round-trips drained.

- New `fan_out_message_posted` runs the per-recipient `notify` calls with bounded concurrency
  (`buffer_unordered`, cap 8) — the Cluster-199 pattern. Wall-clock for the fan-out collapses from
  `Σ` toward `ceil(N/8)`, and the consumer is no longer serially blocked on one popular message.

## Surprises / decisions

- **Order-independent, so `buffer_unordered`.** Each write targets a distinct recipient row; there
  is no ordering constraint, so the unordered combinator (vs `buffered`) is the right fit. A store
  error on any recipient still short-circuits, matching the prior `?`-in-loop behaviour.
- **Function-scoped `futures` imports.** The module already imports `tokio_stream::StreamExt` for
  the bus `.next()`; a module-level `futures::StreamExt` made `.map`/`.next` ambiguous (E0034). The
  helper scopes the `futures` traits to itself, and maps on the **iterator** (`Iterator::map`)
  before `stream::iter`, so the stream only ever sees the unambiguous `buffer_unordered`/
  `try_collect`.
- **Concurrency, not batching.** This de-serializes the fan-out (the "serial round-trips" the audit
  flagged) with an established, low-risk pattern. Collapsing the `2N` round-trips into a couple of
  batched statements (a batch mute-filter + a multi-row `INSERT … ON CONFLICT DO NOTHING`) is a
  further optimization, logged in Open Work — it needs new both-backend batch store methods and
  the email side-effect keyed off the `RETURNING` set, so it is its own cluster.

## Test evidence

`notification_router_e2e` (incl. the follow-fanout case), `notifications_inbox_e2e`,
`follows_rest_e2e`, `digest_sweeper_e2e` — all green (behaviour-preserved). fmt + strict clippy +
`--all-targets` + bootstrap-strip clean.

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI); P2 code-side —
notification batch-insert (the further optimization above), projector link-management surface,
Store trait split, and the MCP `post_message` slash-dispatch decision (needs a product call:
implement parity vs document the intentional difference).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
