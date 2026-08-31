# Cluster 343.0 retro — keyset-paginate the channel thread list (audit P2)

> Tag **`v343.0.0`**. Phase XXIV (post-gate hardening). **Cluster 12 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The last unpaginated list endpoint. `GET /channels/:cid/threads` and the MCP `list_threads` tool
called `Store::list_threads(channel_id)` — an unbounded full-channel query. A channel with many
threads returned all of them in one response (and one query).

- **`Store::page_threads_for_channel(channel_id, after, limit)`** (both backends) — keyset
  `(created_at, id)` ascending, exclusive cursor, `LIMIT` in SQL. The channel-scoped twin of
  `page_threads_for_workspace`.
- REST `GET /channels/:cid/threads` gained `ListThreadsQuery { limit?, cursor? }` (default 100,
  clamp 1..=500); MCP `list_threads` gained `limit`/`cursor` args. Postgres routes it through the
  read replica (`read_pool()`), like the read it replaces.
- The unbounded `list_threads` **stays** for internal full-list callers (workspace-context
  assembly, workspace import) that genuinely need every thread.

## Surprises / decisions

- **New method, not a signature change.** Making `list_threads` itself paginated would have
  silently truncated the internal callers that need all threads. A separate paginated method leaves
  those correct and bounds only the API-facing path — the same split the codebase already uses for
  workspace threads (`list_threads_for_workspace` vs `page_threads_for_workspace`).
- **Ordering flips to ascending.** The old plain list was `created_at DESC` and unbounded; the page
  is `created_at ASC` (keyset-stable), consistent with `page_threads_for_workspace` and the
  `list_messages`/context pagination already documented in `Integration.md`. Pre-launch, the
  behaviour change is acceptable and is the correct scalable contract.
- **Tombstone filter is defensive.** `page_for_channel` filters `tombstoned_at IS NULL` to match
  `page_for_workspace`, though no public store method tombstones a thread today (so the filter is
  not exercised by a test, same as the workspace variant).

## Test evidence

`assert_channel_thread_pagination` in the both-backend `bulk_reads` suite (keyset walk reproduces
the order once, cursor exclusive, channel scoping holds). `openapi_e2e` bijection green with the new
`IntoParams` query; `maidan-mcp` lib (61) + `event_emission`/`channel_access`/`scheduler` e2es
green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Remaining audit items: **P1.5** (egress wire-path tests + LSN replica CI) and the remaining P2
code-side items (projector link-management surface, notification-router fan-out, Store trait split,
MCP `post_message` slash-dispatch decision).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
