# Cluster 254.0 retro — the shape of a digest

> Tag **`v254.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 18** — Arc I.

## What shipped

- `EmailDeliveryMode` (`Immediate` default / `Digest`) + `DigestDue` in
  maidan-types; `maidan_member_delivery_prefs` + `maidan_member_digest_state` (pg
  0048 / sqlite 0047); store `set_delivery_mode` / `get_delivery_mode` /
  `set_last_digest_at` / `members_due_for_digest`, both backends. The store
  foundation for scheduled digests — the alternative-mode product the user chose
  (immediate *or* digest, not both) needs a per-member mode plus a digest watermark.

## Surprises / decisions

- **Default via row-absence keeps the change dark.** `get_delivery_mode` returns
  `Immediate` when no `maidan_member_delivery_prefs` row exists, so every existing
  member is unaffected until they explicitly choose `Digest`. No backfill, no
  behaviour change — the foundation is inert by construction.
- **A watermark, not a per-notification flag.** The temptation is to mark
  notifications "included in a digest," but that's a write per row per digest on the
  hot `maidan_notifications` table. A single `last_digest_at` per member turns "what's
  unread since the last digest" into a cheap indexed range and leaves the
  notification rows alone.
- **The enumeration carries the address.** `members_due_for_digest` already has to
  JOIN `maidan_member_emails` (an addressless member can't be digested), so returning
  the email in `DigestDue` costs nothing and saves the sweeper a per-member round-trip.
- **The SQLite datetime-format trap, avoided up front.** `created_at` is stored via
  `datetime('now')` (space-separated, no zone) while `last_digest_at` is bound as
  rfc3339 (`T`, `+00:00`). A raw string `>` would compare them wrong; wrapping both
  in `datetime(...)` normalizes to the canonical form. Postgres has real
  `timestamptz`, so it just compares — with `'epoch'` as the never-digested floor.
- **`clippy::derivable_impls` caught the manual `Default`.** A hand-written `impl
  Default for EmailDeliveryMode` is a lint error under `-D warnings`; the fix is
  `#[derive(Default)]` + `#[default]` on `Immediate`. (Local clippy caught it before
  CI — same class as the strict-unwrap lesson: run the linters, not just tests.)

## Capability table extension

| Change | Where |
|--------|-------|
| `EmailDeliveryMode` + `DigestDue` | `maidan-types/src/models.rs` |
| `maidan_member_delivery_prefs` + `maidan_member_digest_state` + `email_digest` store (both backends) | `migrations/*`, `store/*/email_digest.rs` |

## Risks identified + still open

- None — new tables + types off every existing path; nothing reads or writes them
  yet.

## Forward look

**255** wires it: the router skips a digest-mode member's immediate email (they get a
digest instead), and an opt-in digest sweeper worker (`MAIDAN_DIGEST_TICK_SECS`,
reuse the Cluster-227 scheduler-sweeper shape) drains `members_due_for_digest`,
emails each an unread rollup, and advances `set_last_digest_at`. Then REST (256) +
MCP (257) to set the delivery mode. Then **Program D**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 253.0]].
