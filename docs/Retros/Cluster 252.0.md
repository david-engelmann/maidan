# Cluster 252.0 retro — remembering when you were here

> Tag **`v252.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 16** — Arc I.

## What shipped

- `maidan_member_last_seen` (pg 0047 / sqlite 0046; `member_id` PK, `last_seen_at`)
  + store `touch_member_last_seen` (upsert `now()`) / `get_member_last_seen` →
  `Option<DateTime<Utc>>`, both backends. The durable presence signal that
  presence-aware email routing (Cluster 253) needs — storage only, unwired.

## Surprises / decisions

- **A table again, not a member column.** Identical reasoning to the Cluster-248
  email store: `last_seen_at` reads like member metadata, but a column on
  `maidan_members` has to be threaded through every `row_to_member` and every member
  `SELECT`/`RETURNING` across both backends (the `maidan-schema-column-ripple` trap).
  A one-row-per-member side table is zero-ripple and keeps the hot member row lean —
  and `last_seen` is written on *every* connect, so keeping that churn off the
  members row is a bonus, not just a convenience.
- **No `MemberLastSeen` model.** The row is `(member_id, last_seen_at)` and every
  reader wants only the timestamp, so `get` returns `Option<DateTime<Utc>>` straight.
  A wrapper struct would be pure ceremony.
- **`now()` lives in the store.** `touch` computes the instant server-side
  (pg `NOW()`, sqlite a bound rfc3339 `now`) so the eventual WS caller passes only a
  member id — the same shape as the `claim_next` lease timestamps.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_member_last_seen` + `touch`/`get` store, both backends | `migrations/*`, `store/*/member_last_seen.rs` |

## Risks identified + still open

- None — a new table off every existing path; nothing reads or writes it yet.

## Forward look

**253** wires it: call `touch_member_last_seen` at the WS presence-registration site
(`PresenceHub::register`), and add a presence-aware guard to the router's
`deliver_notification_email` — skip the email when `now - last_seen` is inside an
env-configurable "recently active" window (they're already here; don't email). Then
scheduled **digests** (unread rollups), and — optionally — MCP email tools for
parity. Then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 251.0]].
