# Cluster 304.0 retro — durable mail outbox foundation

> Tag **`v304.0.0`**. Phase XXIV (post-gate hardening). Durable mail retry queue, part 1. No new gate tag.

## What shipped

The zero-blast-radius store foundation for a durable, retrying notification-email path
(replacing the best-effort fire-and-forget send at `notification_router.rs:334`):

- **`maidan_mail_outbox` table** (pg 0050 / sqlite 0049) — `id`, `to_address`, `subject`,
  `body`, `status` (`pending`/`delivered`/`dead`), `attempts`, `next_attempt_at`, `last_error`,
  timestamps; a partial index on `(next_attempt_at) WHERE status='pending'` for the claim query.
- **`MailOutbox` / `NewMailOutbox` models + `MailOutboxId`** (maidan-types). `MailOutbox` is
  content-only (`id`, `to_address`, `subject`, `body`, `attempts`) — the scheduling/status
  columns stay internal to the store, so there's no shared row-mapping to ripple.
- **Store methods, both backends** (`store/{postgres,sqlite}/mail_outbox.rs` + 5 trait methods):
  `enqueue_mail`; `claim_next_due_mail(now, lease_secs)` — atomically leases the oldest due
  `pending` row (bumps `attempts`, pushes `next_attempt_at` forward by the lease); `mark_mail_delivered`;
  `mark_mail_failed(id, error, retry_at)` — `Some(t)` reschedules, `None` dead-letters;
  `count_dead_mail` (DLQ depth). Postgres claim uses `FOR UPDATE SKIP LOCKED`; SQLite uses a
  serialized select-then-update tx (the Cluster-227 scheduler pattern).

No worker, no router change, no routes — foundation only (the 159/217/226/230/234 pattern).

## Surprises / decisions

- **Leased claim, not a status flag.** `claim_next_due_mail` pushes `next_attempt_at` forward
  by `lease_secs` and bumps `attempts` in the claim itself, so a worker that crashes mid-send
  releases the row after the lease and the next claim retries it — **at-least-once**, a
  duplicate email being low-harm (the same polarity call as the Cluster-255 digest sweeper).
  No separate "sending" state to get stuck.
- **`attempts` is `BIGINT` in Postgres** (not `INTEGER`) so it decodes to the model's `i64`
  directly — pg `INTEGER` is `i32` and would need a cast; SQLite `INTEGER` is already 64-bit.
- **Worker decides retry-vs-dead, store just executes.** `mark_mail_failed` takes an explicit
  `retry_at: Option` rather than a max-attempts policy — the backoff schedule + attempt ceiling
  live in the worker (305), keeping the store mechanism-only and the policy testable in one place.
- **All timestamps store-bound rfc3339** on SQLite, so a plain `next_attempt_at <= ?` compare is
  consistent (no `datetime()` wrapping needed, unlike the Cluster-254 digest cross-format trap).

## Capability table extension

New `maidan_mail_outbox` table + store (enqueue / leased-claim / delivered / retry-or-dead /
DLQ-count), both backends. No server behavior change yet.

## Risks identified + still open

- Foundation only — nothing enqueues or drains it yet. The router still sends best-effort
  until **305**. No blast radius.

## Forward look

**305** wires it: the router *enqueues* to the outbox (after the digest/presence checks — only
what it would actually send) instead of the best-effort spawn+send, and a background
`mail_worker` drains due entries via `state.mail` with exponential backoff + dead-lettering.
**306** adds a DLQ ops read (list/retry dead mail).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens the durable-mail-retry arc
(Open Work "durable mail retry queue"). Follows the MCP `2026-07-28` arc ([[Retros/Cluster 303.0]]).
