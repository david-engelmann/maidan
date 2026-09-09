# Cluster 359 retro — inbox & search depth (Wave 1 #10, N2 / N5 / N4)

Wave 1 #10 bundles three notification/search refinements: the flat inbox floods
and the digest is a bare count, and search can't scope by time. This cluster
makes the inbox *legible* (group by thread, snooze the noise, surface the
decisions you missed) and search *time-scopable* (`during:`).

## What shipped

- **359.1 (#663) — N4: date-range search.** `SearchFilters {after, before}` — a
  half-open `[after, before)` window on `posted_at`, both backends × lexical +
  semantic (hybrid inherits it), over REST `GET …/search` + MCP `search_messages`.
  Additive to the existing route/tool — no new route.
- **359.2 (#664) — N5a: notification snooze.** A `snoozed_until` column on
  `maidan_notifications` (pg 0065 / sqlite 0064); the list + unread-count queries
  exclude currently-snoozed rows, so a snoozed notification drops out of the inbox
  + badge and resurfaces automatically. `Store::snooze_notification` + REST
  `POST /members/:id/notifications/:nid/snooze` + MCP.
- **359.3 (#665) — N5b: inbox grouped by thread.** `group_notifications_by_thread`
  — a pure grouping (the Cluster-197 `tool_transcript` pattern, no SQL group-by)
  over the snooze-filtered list → one `NotificationThreadGroup` per thread
  (count / unread_count / latest), newest-activity first, over REST + MCP.
- **359.4 (#666) — N2: buried-decisions digest.** The digest leads with the
  *decisions* a member missed — task results (Cluster-234 `ThreadResult`) produced
  by someone else in a channel/thread the member follows, since their last digest
  — instead of a bare unread count. `Store::buried_decisions_for_member` +
  `last_digest_at` on `DigestDue`; the sweeper composes the decision list; REST
  `GET /members/:id/decisions` + MCP `list_buried_decisions`.

## Decisions

- **Grouping is a pure function, not SQL.** Fetch the (snooze-filtered) list, then
  group in Rust — the Cluster-197 pattern. A backend GROUP-BY-with-latest needs a
  window function or correlated subquery per backend; the pure fn is simpler, DRY
  across REST + MCP, and unit-testable without a DB.
- **Snooze is a column, not a side table.** `maidan_notifications` has its own
  local `row_to_notification` (not a shared hot-path mapper), so the column ripple
  is contained to the one module. Snooze is intrinsic + one-to-one with a
  notification — a column, not a side table (unlike the 356/357 mute tables, which
  are member×scope).
- **"Decision" == a task result.** The current model's closest signal to a
  "decision" is a Cluster-234 `ThreadResult` (the structured outcome of a task).
  "Buried" = produced by someone else, in a channel/thread the member follows,
  since their digest watermark — reusing the follows infra (244) + the digest
  watermark (254).
- **Date range is half-open `[after, before)`.** So adjacent day-ranges don't
  double-count the midnight boundary. Bound only the dimensions given.

## Surprises

- **The SQLite notification INSERTs list `created_at, read_at` explicitly** (the
  Postgres ones default them), so the `read_at`→`, snoozed_until` column-list sed
  added the column to the INSERT lists too — "11 values for 12 columns" until the
  three sqlite `VALUES` (create / create_if_absent / batch) got their `, NULL`.
  Postgres was unaffected (its INSERTs stop at `actor_id`).
- **A test closure taking `&dyn Store` into an `async move`** tripped the same
  lifetime error as prior clusters — inlined it instead (a `|list: &[_]| …any()`
  closure over the already-fetched list is fine; a `&store`-capturing async one is
  not).

## Test evidence

- Store: `follows` (snooze on the notifications suite), `buried_decisions` (follow
  scoping + watermark + thread-follow), all both backends; the pure
  `group_notifications_by_thread` + `SearchFilters::is_empty` + date-range unit
  tests; the shared search `assert_date_range_filter` (both backends).
- Integration: `digest_leads_with_buried_decisions` (the digest email lists the
  decision); REST inbox e2e (snooze + grouped); openapi + capability-matrix + both
  MCP contract-sync + backend-parity, all green per PR.

## Forward look

The inbox is now legible — group, snooze, decisions — and search is
time-scopable. **Deferred / follow-ups (logged in Open Work):** the semantic
date-range path is covered by the shared clause + compile, not a dedicated
embeddings test; the grouped inbox scans a bounded page (no cross-page grouping);
"decision" is `ThreadResult`-only (a `result_kind=decision|plan|merge_authorized`
facet — Open Work #24 — would sharpen it); a snoozed-item *view* (see-what-I-
snoozed) is not surfaced (they resurface on lapse). Next-ranked is **Wave 1 #11 —
G-dev-1** (a frozen, token-budgeted context pack).

## Acknowledgements

Built as a five-PR run (#663 → #667) on the Cluster-234 thread results, the
Cluster-238/245 notification router + follows, the Cluster-254 digest, and the
v1.2.2 search facets — each rebased onto `main` as its parent merged.
