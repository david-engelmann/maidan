# Cluster 394 retro — Wave 3 #34: tombstone explorer, backlink index, kind census

Wave 3 #34 (B8 + B27) asked for a **tombstone/deletion explorer**, a
**backlink index** ("what points at this message"), and a **kind
census**. Three read surfaces over existing rows — no new table.

This is **not** hash-chain verify (Cluster 392), **not** snapshot/tap
(Cluster 393), **not** named capability sets / `maidan://` URIs (#35),
and **not** WASI slash-handlers (#36).

Three impl PRs (394.1–394.3) + this retro. Every PR targets `main`.
**Row #34 is closed.** Do not start Wave 3 #35–36 from this close.

## What shipped

- **394.1 (#859) — types + store.** `TombstoneRecord` / `MessageBacklinks`
  / `KindCensus` in `maidan-types`. `IntegrityStore` on `Store` (blanket
  + both backends). Soft-deleted messages plus optional hard-purge
  reconstructions from `MessageTombstoned`. Backlinks = Cluster 320
  `list_references_to` plus pins, reactions, and votes. Census is
  `GROUP BY` event kind with a `deny_channels` pre-filter. Postgres
  reads use `read_pool()`.
- **394.2 (#860) — REST.** `GET /workspaces/:id/tombstones` (optional
  `include_purged` + channel/thread scope), `GET /messages/:id/backlinks`,
  `GET /workspaces/:id/kind-census`. `workspace:read`. OpenAPI +
  capability-map preflight. `explorer_e2e` (minted bearer).
- **394.3 (#861) — MCP.** `list_tombstones` / `list_message_backlinks` /
  `get_kind_census`. Standard 5-place wiring. Combined pre-dispatch arm
  for optional `channel_id` + `thread_id`.

## Decisions

- **No new table.** The explorer, the backlink index, and the census
  are queries over rows that already exist. A second graph would
  duplicate Cluster 320 reverse edges.
- **Only messages tombstone.** Threads, channels, and members carry a
  `tombstoned_at` column but have no tombstone API. `TombstoneEntityKind`
  is a closed enum so a later entity is an additive variant.
- **Hard purge is the honest deletion trail.** Soft-delete keeps the
  row and clears `body`/`content`. `include_purged` reconstructs the
  missing row from `MessageTombstoned`. The body is gone either way —
  this surface does not restore content.
- **Mentions are outgoing.** A mention is the message pointing at a
  member. The backlink index is incoming pointers only.
- **Census deny-set keeps `channel_id IS NULL` workspace-level events.**
  Same `private_channel_deny_set` as filtered-ANN search (Cluster 200).
  The tombstone list still post-filters by `can_access_thread` because
  the store cannot see DM participation.
- **Backlinks 404 after hard purge.** There is no row to authorize.
  A retained tombstone still answers (the row is there, body cleared).

## Surprises

- **`MessageBacklinks` cannot derive `PartialEq`.** `Reference` / `Pin`
  / `Reaction` / `Vote` do not. Do not add `PartialEq` to those types
  for a test convenience.
- **A tool with both optional `channel_id` and `thread_id` cannot join
  two exclusive `enforce_channel_access` match arms.** The first arm
  wins and the thread gate never runs. Combined arm is required.
- **MCP maps a missing message through `AuthError::Store` →
  `Internal("not found")`.** REST maps the same store miss to 404.
  Pre-existing; not changed here.
- **Postgres store tests skip when Docker is unavailable.** Graceful
  skip, same as the rest of the suite.

## Test evidence

- Types: `TombstoneEntityKind` wire `message`; `clamp_tombstone_limit`
  1..=500 default 100; census omits empty scope ids.
- Store (sqlite always; postgres when Docker): retained tombstone;
  `include_purged` after `purge_message`; backlinks (ref + pin +
  reaction + vote); census includes `message_posted` / `workspace_created`.
- REST e2e: 401 unauth; private-channel tombstone does not leak;
  `include_purged` reconstructs; backlinks shape; 404 after purge;
  census totals.
- MCP: same fixture via `from_session`; catalog + capability-map
  contracts stay in sync.
- OpenAPI bijection + HTTP capability matrix for the three GETs.

## Forward look

**Cluster 394 is complete. Row #34 is closed.**

Do not start Wave 3 #35–36 from this retro. #35 is named capability
sets + `maidan://` URIs; #36 is a WASI slash-handler kind. Cluster 393
(#33 snapshot/tap) is already on `main` — do not reopen it from here.

Deferred: `/ui` reference-graph view (Program U — the index is the
backend; the pane is not); mentions as backlinks; thread/channel/member
tombstone APIs; a dedicated `workspace:export`-style cap (reads stay
`workspace:read`).

Do **not** cut `v394.0.0` from this PR — the maintainer tags, which
triggers `release.yml`.

## Acknowledgements

#859 types+store → #860 REST → #861 MCP → this retro.
