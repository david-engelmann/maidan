# Cluster 393 retro — Wave 3 #33: snapshot catch-up + tap projector contract

Wave 3 #33 (B18 + B19) asked for a **snapshot + since-LSN catch-up**
(getRepo-shaped) and a **tap projector contract** (verify, backfill,
filter, live-waits-for-history, webhook/WS). Search is a projector; it
must not diverge from the log silently.

This complements Cluster 392. The hash chain verifies the **retained
suffix**. Snapshot catch-up covers the **pruned prefix** so a peer that
never saw aged-out events can resume without trusting the host for
history the log no longer holds.

Four impl PRs (393.1–393.4) + this retro. Every PR targets `main`.
**Row #33 is closed.** Do not start Wave 3 #34–36 from this close.

Do **not** cut `v393.0.0` from this PR — the maintainer tags.

## What shipped

- **393.1 (#854) — types.** `$type` `maidan.event-log.snapshot/1` and
  `maidan.event-log.catch-up/1`. `SnapshotGraph` is the Cluster 187/391
  export graph minus `exported_at` so two snapshots of the same tables
  hash the same. `verify_snapshot` / `verify_catch_up` / `catch_up_allowed`
  fail closed. Tap contract types: `TapSurface`, `TapContract`,
  `TapFault`, `SEARCH_PROJECTOR_KINDS`, `history_caught_up` (workspace
  or shape head, **not** the global Room-LSN).
- **393.2 (#855) — store.** Both backends: `workspace_event_floor` /
  `head` / `at_or_before`. `build_log_snapshot` + `catch_up_since`
  (limit clamp 1..=500, `ensure_cursor_fresh`, predecessor = this
  workspace's id ≤ `after_lsn`). Tests: snapshot→catch-up, tamper
  fail-closed, pruned prefix → `CursorTooOld`.
- **393.3 (#856) — REST + MCP.** `GET /workspaces/:wid/snapshot`
  (`include_graph` default false). Header + `graph_hash` is
  `workspace:read` / federation peer; `include_graph=true` is
  `token:admin` or a registered peer. `GET …/events/catch-up` is the
  same auth as `list_events`. CursorTooOld 409s (REST, WS, MCP SSE)
  now carry a `snapshot` href. MCP: `get_log_snapshot`,
  `catch_up_events`, `verify_event_chain`.
- **393.4 (#857) — search tap.** `SearchTap` verifies every backfill
  row on the **per-workspace** chain, then projects only
  `SEARCH_PROJECTOR_KINDS`. `Lagged` without a durable log is
  `RebuildRequired` (error, not warn-and-continue). `Lagged` with a
  log resets the tap and re-verifies the retained suffix.
  `IndexerHandle.rebuild_needed` is the fail-loud rebuild signal.

## Decisions

- **Hashed, not signed.** Cluster 391 already answers authorship
  (Ed25519 export). A snapshot is a content-addressed checkpoint of
  the domain graph plus the retained floor/head `EventLink`. A
  fabricated-but-internally-consistent graph is 391's problem, not
  a second signature scheme.
- **`include_graph` is the export dump.** A full graph is a
  private-channel dump. Default `false` so `workspace:read` can still
  take a verifiable header + `graph_hash`. The bytes require
  `token:admin` or a federation peer.
- **Catch-up predecessor is tenant-local.** Event ids have
  cross-workspace gaps. The predecessor of `after_lsn` is this
  workspace's latest id ≤ that cursor, not `after_lsn` itself.
- **Live-waits compares the workspace (or shape) head**, not the
  global `Maidan-Room-LSN`. Other tenants move the global watermark.
- **A pruned gap never clamps.** Cluster 388 `CursorTooOld` → refetch
  the snapshot. Search must not keep projecting a gapped suffix.
- **A global backfill page is not one chain.** `prev_hash` is
  per-workspace. The search tap keeps a last `EventLink` per
  `WorkspaceId`. Filter for projection; verify every kind.
- **Did not reopen Cluster 392.** No edits to `event_chain.rs` or
  the 392.4 claim_next / A2A pin files.

## Surprises

- Synthetic live `BusEnvelope` events use `log_id = 0`. Skipping
  `log_id <= watermark` unconditionally dropped every live event
  after a zero-start watermark. Live-skip is only safe when a
  durable log is attached.
- `verify_catch_up(None, &[row])` is the correct first event of a
  workspace (or a pruned floor): `verify_link` with `previous=None`
  and `from_genesis=false` checks `content_hash` only.
- Tampering `s1` instead of `s2.prev_hash` produced `IdNotIncreasing`,
  not `PrevHashMismatch`. The fail-closed test had to break the
  successor's `prev_hash`.
- MCP tool handlers take `&Arc<dyn Store>`, not `&dyn Store` —
  dispatch already holds the Arc.
- The 393.2 postgres helper dropped the testcontainer before the
  suite ran (`PoolTimedOut` on CI). The container must stay in the
  test scope — same as `event_chain.rs`.

## Test evidence

- Types: snapshot hash omit-`exported_at`; catch-up allowed / too-old;
  chain-from-predecessor; `history_caught_up` vs a higher global head;
  tap contract defaults for search kinds.
- Store (SQLite + Postgres testcontainers skip): snapshot then
  catch-up; tampered successor fail-closed; pruned prefix
  `CursorTooOld`.
- Server e2e: header-only snapshot for `workspace:read`;
  `include_graph` denied without `token:admin`; catch-up pages;
  CursorTooOld 409 carries `/workspaces/{id}/snapshot` on REST / WS /
  MCP SSE; broken chain is 409 `event-log-broken`.
- Search: backfill projects `MessagePosted`; tamper fails closed
  (no later message projected); indexer with a log backfills before
  live. Existing indexer observe/filter tests stay green once
  live-skip is log-gated.
- Contracts: OpenAPI bijection, HTTP capability map, MCP tool names +
  capability map (sorted).

## Forward look

**Cluster 393 is complete. Row #33 is closed.**

Do not start Wave 3 #34–36 from this retro. #34 is a tombstone /
deletion explorer + backlink index; #35 named capability sets +
`maidan://` URIs; #36 WASI slash-handler. Cluster 392 (hash-chained
log, row #32) is a sibling — do not reopen its design from here.

Deferred:

- MST/CAR (explicitly not this row).
- Streaming / chunked snapshot graphs for huge workspaces (same
  class as 391's paginated-export deferral).
- Reactions / votes / artifact blobs in the snapshot graph (187/391
  gaps — the graph matches the export).
- A dedicated operator "rebuild search from messages" job beyond
  the existing reindex path; 393.4 only fails loud and sets
  `rebuild_needed`.
- Webhook / WS / AG-UI do not re-verify the hash chain on every
  live frame (Cluster 388 HTTP-then-WS + Lagged→log already bind
  those surfaces; the named contract lives in `tap.rs`).

Do **not** cut `v393.0.0` from this PR.

## Acknowledgements

#854 types → #855 store → #856 REST/MCP → #857 search tap → this retro.
