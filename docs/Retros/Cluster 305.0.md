# Cluster 305.0 retro — mail-outbox worker + router enqueue

> Tag **`v305.0.0`**. Phase XXIV (post-gate hardening). Durable mail retry queue, part 2. No new gate tag.

## What shipped

The notification-email path is now durable end-to-end — the router enqueues, a background worker
delivers with retry/backoff/dead-lettering (the Cluster-304 outbox comes alive):

- **Router enqueues** (`notification_router.rs`): `deliver_notification_email` keeps every
  suppression check (transport-configured, address-on-file, digest-mode, presence-window) — the
  gates that decide *whether* to send — but its final step is now `enqueue_mail(...)` instead of a
  best-effort `mail.send`. A transient SMTP failure is no longer logged-and-dropped. Metric
  outcome `enqueued`.
- **`mail_worker` (`mail_worker.rs`)**: each tick, `sweep_once` claims up to 1000 due entries
  (`claim_next_due_mail`, a leased atomic claim) and sends each via the transport →
  `mark_mail_delivered` (`sent`), or on failure either **reschedules** with exponential backoff
  (`retry`; base 30s, ×2, cap 1h) or **dead-letters** once `attempts >= MAX_ATTEMPTS` (8) (`dead`).
  A `MailSweepStats` return makes it directly testable.
- **Spawned in `main.rs` whenever a transport is configured** (`state.mail.is_some()`) — the
  router only enqueues then, so the two are paired; tick default 5s, `MAIDAN_MAIL_WORKER_TICK_SECS`.

## Surprises / decisions

- **Worker runs when mail is configured, not behind a separate opt-in env.** Unlike the digest
  sweeper (opt-in tick), a queue with a sender but no drainer would just pile up — so the worker
  is spawned exactly when a transport exists. Tests (no transport) never enqueue and never spawn it.
- **The suppression gates stay in the router, pre-enqueue.** digest-mode / presence / no-address
  members are filtered *before* enqueue, so the outbox only ever holds mail that should actually
  send — the worker has no policy, just delivery + retry. This kept the three existing
  router/presence/digest e2es intact by inserting one `mail_worker::sweep_once` before their
  `mailer.sent` assertions (the skipped members simply never enqueue).
- **Multi-replica safe.** The leased `FOR UPDATE SKIP LOCKED` claim (304) hands each replica a
  distinct row, so — unlike the deliberately-single-flight digest sweeper — the mail worker can run
  on every replica.
- **Retry-not-drop is the headline.** The e2e proves a failed send reschedules (backoff), an
  immediate re-sweep is a no-op (leased forward), and the mail is neither dropped nor
  dead-lettered on a single failure. Full dead-letter is covered by the 304 store test + the
  backoff unit test (an 8-attempt worker e2e would need to fast-forward the backoff clock).

## Capability table extension

Notification email is now durable: router enqueues → `mail_worker` delivers with retry/backoff +
dead-lettering, replacing the best-effort send. New `maidan_email_delivered_total` outcomes
(`enqueued`/`sent`/`retry`/`dead`). No new capability/route.

## Risks identified + still open

- Dead-lettered mail accumulates in `maidan_mail_outbox` with `status='dead'` — visible via
  `count_dead_mail`; a **DLQ ops read** (list/retry) is **306**. Retention pruning of terminal
  outbox rows is a follow-up (the Cluster-186 sweeper doesn't cover this table yet).

## Forward look

**306** closes the arc: a DLQ ops read (list dead mail + a retry/requeue control), so an operator
can see and recover permanently-failed sends.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 304.0]]. Brings
the durable outbox online; the digest sweeper ([[Retros/Cluster 255.0]] pattern) was the worker
template.
