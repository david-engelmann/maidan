# Cluster 254.0 — email digest data model (store foundation)

> **Program C (notifications & reach), part 18** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v254.0.0`**. No new gate tag.

## Goal

The store foundation for **scheduled email digests**. The chosen product model
(alternative-mode): a member picks *either* immediate per-notification emails
(Cluster 249) *or* a periodic digest — not both. This cluster lands the data model
that choice needs, with zero wiring.

## Scope

| Change | Where |
|--------|-------|
| `EmailDeliveryMode` enum (`Immediate` default / `Digest`) + `DigestDue` struct | `maidan-types/src/models.rs` |
| `maidan_member_delivery_prefs` + `maidan_member_digest_state` tables (pg 0048 / sqlite 0047) | `migrations/*`, `migrate.rs` |
| Store `set_delivery_mode` / `get_delivery_mode` / `set_last_digest_at` / `members_due_for_digest`, both backends | `store.rs`, `store/{sqlite,postgres}/email_digest.rs`, `store/*/mod.rs` |

## Design decisions

- **Delivery mode as a separate pref table, default via row-absence.** An absent
  `maidan_member_delivery_prefs` row means `Immediate` — the Cluster-249 behaviour —
  so existing members need no backfill. A dedicated table (not a column on
  `maidan_member_emails`) keeps the Cluster-248 `MemberEmail` mapping untouched.
- **A digest watermark, not a "digested" flag on notifications.** `maidan_member_
  digest_state.last_digest_at` records when a member was last sent a digest; the
  enumeration counts unread notifications *created since*. This keeps the watermark
  off the hot `maidan_notifications` rows and makes "what's new since last digest" a
  cheap range, not a per-row mutation.
- **`members_due_for_digest` returns the address inline.** The enumeration already
  JOINs `maidan_member_emails`, so it returns `DigestDue { member_id, email,
  unread_count }` — the sweeper (Cluster 255) needs no extra per-member lookup.
- **SQLite datetime comparison is normalized.** `maidan_notifications.created_at` is
  written as `datetime('now')` (`YYYY-MM-DD HH:MM:SS`) while `last_digest_at` is
  bound as rfc3339; the query wraps both in `datetime(...)` so they compare
  correctly. Postgres uses native `timestamptz` with an `'epoch'` never-digested
  floor.
- **Foundation only.** No router change, no sweeper, no routes — nothing reads or
  writes these tables yet. Zero behaviour change until 255.

## Non-goals / deferred

- **Router honoring the mode + the digest sweeper worker** (Cluster 255) — the
  router skips digest-mode members' immediate emails; an opt-in sweeper rolls up
  their unreads and advances the watermark.
- **REST / MCP to set the delivery mode** (Clusters 256 / 257).

## Risks

- Migration registration — covered by the both-backend store test + `dialect_parity`
  / `concurrent_migrations`.
