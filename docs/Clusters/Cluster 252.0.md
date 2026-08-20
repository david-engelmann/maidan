# Cluster 252.0 — durable member last-seen (store foundation)

> **Program C (notifications & reach), part 16** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v252.0.0`**. No new gate tag.

## Goal

The prerequisite for **presence-aware routing** (Cluster 253): a durable record of
when each member was last connected. Presence is in-memory only today
(`presence.rs`, a per-process `PresenceHub`), so it can't answer "was this member
recently active?" after a restart or across replicas. A tiny `last_seen` table gives
the email-delivery step a persistent signal to decide "skip the email — they're
here" vs "they're away, send it."

## Scope

| Change | Where |
|--------|-------|
| `maidan_member_last_seen` table (pg 0047 / sqlite 0046; `member_id` PK, `last_seen_at`) | `migrations/*`, `migrate.rs` |
| Store `touch_member_last_seen` (upsert `now()`) / `get_member_last_seen` → `Option<DateTime<Utc>>`, both backends | `store.rs`, `store/{sqlite,postgres}/member_last_seen.rs`, `store/*/mod.rs` |

## Design decisions

- **A separate table, not a column on `maidan_members`.** The same call as the
  Cluster-248 email store: a `last_seen_at` column on the members row would ripple
  through every `row_to_member` / member `SELECT` crate-wide (the
  `maidan-schema-column-ripple` lesson). A one-row-per-member
  `maidan_member_last_seen` table is zero-ripple.
- **No new model type.** `get` returns `Option<DateTime<Utc>>` directly — the row is
  just `(member_id, last_seen_at)` and the caller only ever wants the timestamp.
  Adding a `MemberLastSeen` struct would be ceremony for a two-column row.
- **`touch` is an idempotent upsert to `now()`.** The write path (Cluster 253, at the
  WS presence-registration site) calls `touch` on connect; repeated touches just move
  the timestamp forward. `now()` is computed in-store (pg `NOW()` / sqlite bound
  rfc3339) so the caller passes only the member id.
- **Foundation only.** No wiring — nothing calls `touch` yet, and nothing reads
  `get`. Zero behaviour change until 253.

## Non-goals / deferred

- **Wiring** (Cluster 253) — `touch` at the presence-registration site + a
  presence-aware guard in `deliver_notification_email` (skip the email when
  `now - last_seen < window`, env-configurable threshold).
- Digests (scheduled unread rollups) — a later Arc-I cluster.
- Any presence-history / session-duration analytics — this is a single latest-seen
  instant, not a log.

## Risks

- Migration registration — covered by the both-backend store test +
  `dialect_parity` / `concurrent_migrations`.
