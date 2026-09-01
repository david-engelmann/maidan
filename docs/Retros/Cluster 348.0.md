# Cluster 348.0 retro — batch the notification fan-out mute check (audit P2)

> Tag **`v348.0.0`**. Phase XXIV (post-gate hardening). **Cluster 17 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The follow-up to Cluster 344 (which de-serialized the fan-out with bounded concurrency). A
`MessagePosted` fanned out to its followers still ran **one `is_notification_muted` query per
recipient** — `2 × followers` store round-trips.

- **`Store::filter_muted_members(kind, &[MemberId])`** (both backends — SQLite a dynamic
  `IN (…)`, Postgres `= ANY($2)`) resolves the muted subset of a recipient set in **one query**.
- The `MessagePosted` fan-out now batch-fetches the muted set once, meters the suppressed, and
  writes only the unmuted (concurrently, per 344). Cuts the fan-out from `2 × followers` round
  trips toward `followers + 1`.
- `notify`'s insert / email / metric tail is extracted into **`write_notification`**, shared by
  the batched fan-out and the single-recipient mention path (which keeps its own `is_muted` check).

## Surprises / decisions

- **The mute-filter is the low-risk half; the write batch is the fiddly half.** Collapsing the N
  inserts into a single multi-row `INSERT … ON CONFLICT DO NOTHING RETURNING` would take the fan
  out to `2` round-trips, but it needs Postgres `UNNEST` (or numbered-placeholder juggling) and a
  **chunked** dynamic `VALUES` on SQLite (10 columns × N rows brushes the 999-parameter limit) —
  plus the email side-effect keyed off the `RETURNING` set. That is a genuinely riskier change on
  a shipped notification path, so it is logged as its own further optimization; the mute-filter is
  a clean, safe, real win on its own.
- **`ANY($2)` on Postgres, dynamic `IN` on SQLite.** Postgres binds the id array (no dynamic SQL,
  the Cluster-200 deny-set convention); SQLite builds `?,?,…` placeholders and binds each id.
  Empty input short-circuits (no query).
- **The mention path is unchanged.** `MentionRecorded` notifies a single member, so it keeps the
  single `is_notification_muted` check → `write_notification`; only the multi-recipient fan-out
  benefits from the batch.

## Test evidence

`notification_prefs` store suite gains a `filter_muted_members` case (both backends: only the
muted member of a set is returned; a different kind returns none; empty input returns empty).
`notification_router_e2e` (incl. the follow fan-out + the mute case) + `digest_sweeper_e2e` green.
fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Remaining audit items: the notification multi-row batch INSERT (the further optimization above),
the LSN-replica CI job (P1.5 second half), and the Store trait split (large maintainability
refactor, low external value — recommend deferring).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
