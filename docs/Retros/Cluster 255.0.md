# Cluster 255.0 retro — the rollup goes out

> Tag **`v255.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 19** — Arc I.

## What shipped

- **Router honors digest mode:** `deliver_notification_email` now skips the
  immediate email for a member in `Digest` delivery mode (metered
  `maidan_email_delivered_total{outcome="skipped_digest"}`) — the mutual-exclusivity
  half of the alternative-mode product.
- **Digest sweeper worker** (`digest.rs`): opt-in via `MAIDAN_DIGEST_TICK_SECS`,
  drains `members_due_for_digest`, emails each an unread-count rollup, and advances
  `set_last_digest_at` on success. Spawned in `main.rs` next to the retention +
  scheduler sweepers.

The alternative-mode digest now works end-to-end: pick `Digest`, stop getting
per-notification emails, get a periodic rollup instead.

## Surprises / decisions

- **Mode gate before presence gate.** Both live in `deliver_notification_email`. A
  digest-mode member should never get an immediate email regardless of whether
  they're currently active, so the mode check comes first; the presence-window check
  only applies to immediate-mode members. A mode-lookup error falls through to send
  (immediate is the safe default).
- **Advance-after-send, not before.** The scheduler (227) advances its claim
  *before* creating the task thread (at-most-once — a lost firing beats a duplicate
  task). Digests invert that: advance the watermark *after* a successful send, so a
  failure retries and a crash at worst re-sends. For a rollup, a duplicate is trivial
  and a drop means a member silently never hears about those unreads by email — so
  at-least-once is the correct polarity here, opposite the scheduler.
- **Not single-flighting is a choice, not an oversight.** The scheduler went to the
  trouble of `SKIP LOCKED` because a double-fired task thread is real duplicated
  work. A double-sent digest is a minor annoyance, so paying for atomic claim +
  advance (which would also force at-most-once) isn't worth it. The sweeper stays a
  plain enumerate-and-send; the module header tells operators to run it on one
  replica if they want exactly-once.
- **No-op without a transport keeps CI + unconfigured deploys inert.** `sweep_once`
  returns early when `state.mail` is `None`, and the worker only spawns when
  `MAIDAN_DIGEST_TICK_SECS` is set — the same double opt-in as the other sweepers.

## Capability table extension

| Change | Where |
|--------|-------|
| Router skips immediate email for digest-mode members | `notification_router.rs` |
| Digest sweeper worker + `MAIDAN_DIGEST_TICK_SECS` | `digest.rs`, `main.rs`, `lib.rs`, `metrics.rs` |

## Risks identified + still open

- Multi-replica double-send (documented, accepted — low harm; single-replica is the
  guidance for exactly-once).
- The `run` loop has no dedicated e2e; `sweep_once` is covered by `digest_sweeper_e2e`.

## Forward look

**256** REST + **257** MCP to set the delivery mode (self-only, the notification-prefs
cap model) — so a member can choose digest mode over the API instead of out-of-band.
Then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 254.0]].
