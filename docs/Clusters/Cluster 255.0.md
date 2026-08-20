# Cluster 255.0 — digest sweeper + router honors digest mode

> **Program C (notifications & reach), part 19** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v255.0.0`**. No new gate tag.

## Goal

Wire the Cluster-254 digest data model into behaviour: a digest-mode member stops
getting per-notification emails and instead receives a periodic rollup from a
background sweeper. This makes the alternative-mode product real end-to-end.

## Scope

| Change | Where |
|--------|-------|
| Router skips the immediate email for a digest-mode member (metered `skipped_digest`) | `notification_router.rs` |
| Digest sweeper worker: drain `members_due_for_digest`, email a rollup, advance the watermark | `digest.rs` (new), `main.rs`, `lib.rs` |
| `MAIDAN_DIGEST_TICK_SECS` env-gated config; `digest` / `digest_failed` / `skipped_digest` metric outcomes | `digest.rs`, `metrics.rs` |

## Design decisions

- **The mode check lives in `deliver_notification_email`, before the presence
  check.** A digest-mode member never gets an immediate email regardless of
  presence, so the mode gate comes first (after the address lookup). A mode-lookup
  error falls through and sends — the immediate email is the safer default under
  uncertainty.
- **At-least-once, self-healing sends.** The sweeper advances `last_digest_at` only
  *after* a successful send, so a transient SMTP failure retries on the next tick
  instead of dropping the digest. A crash between send and advance re-sends (a
  duplicate rollup) — the right trade for a digest, where a duplicate is a minor
  annoyance and a drop is a lost notification.
- **Deliberately not single-flight across replicas.** Unlike the Cluster-227
  scheduler (whose `SKIP LOCKED` claim prevents a harmful double-fired task thread),
  the digest claim + watermark advance are not atomic, so two replicas both sweeping
  could double-send. A duplicate digest is low-harm, so the sweeper stays simple; the
  operational guidance is to run it on a single replica (the common cron shape) if
  exactly-once matters. Documented in the module header.
- **Opt-in + no-op without a transport.** The sweeper starts only when
  `MAIDAN_DIGEST_TICK_SECS` is set, and `sweep_once` returns early when no
  `MailTransport` is configured — so tests and unconfigured deployments are untouched.

## Non-goals / deferred

- **REST / MCP to set the delivery mode** (Clusters 256 / 257) — until then the mode
  is settable only via the store (or a future surface); a digest-mode member is
  configured out-of-band.
- **Rich digest content** (a list of the actual threads/mentions) — the rollup is an
  unread count today; a fuller digest body is a later refinement.

## Risks

- Multi-replica double-send (documented, accepted — low harm).
- The sweeper `run` loop has no dedicated e2e (it's a `sweep_once` + sleep loop);
  `sweep_once` is covered directly by `digest_sweeper_e2e`.
