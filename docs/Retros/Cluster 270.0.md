# Cluster 270.0 retro — workspace import (REST)

> Tag **`v270.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 4 — workspace import, the route.** No new gate tag.

## What shipped

- `POST /workspaces/import` (`token:admin`) — the write-side inverse of the
  Cluster-187 export, over the Cluster-269 `Store::import_workspace`. The request
  body is exactly the bundle `GET /workspaces/{id}/export` produces (`WorkspaceExport`
  gained `Deserialize`), so export → import is a clean round-trip.
- **Two modes** (`?mode=`):
  - **new** (default) — `import::remap` assigns a fresh id to every entity and
    rewrites all foreign keys, so the content lands as a brand-new workspace. Never
    collides. Returns the fresh workspace id.
  - **restore** — ids preserved verbatim. If a workspace with the bundle's id already
    exists → **409**, unless `&force=true` erases it first (`erase_workspace`) and
    restores over it. For disaster recovery into a fresh database.
- `import::flatten` (export's nested `channels[].members` → the store's flat
  `channels` + `channel_members`) and the pure `import::remap` are unit-tested for
  referential integrity with no database; the route is proven end-to-end
  (`workspace_import_e2e`: export → new → restore-conflict-409 → restore-force).

## Surprises / decisions

- **`remap` is pure, with an injected id source.** `remap(bundle, || Uuid::new_v4())`
  takes the id generator as a closure, so the unit test drives it with real uuids and
  asserts every FK points at a remapped id (workspace, members, channels, threads
  incl. `parent_thread_id`/`assignee_id`, messages, edits, pins, and both reference
  endpoints via a kind-tagged lookup). No DB needed to prove the graph stays
  consistent — the riskiest logic is the cheapest to test.
- **`force`-restore erases the workspace's own tokens.** `erase_workspace` cascades,
  and tokens aren't in the export bundle (secrets are excluded, Cluster 187), so a
  forced restore invalidates the very `token:admin` bearer that authorized it. That is
  correct "replace" semantics — the caller re-provisions credentials after a restore
  — but worth calling out. The request itself is unaffected (auth ran before the
  handler).
- **No `ensure_workspace` on the route.** Unlike export (which pins the token to the
  path workspace), import creates/restores a workspace, so there is no pre-existing id
  to pin to — `token:admin` alone gates it. This matches how `token:admin` is the
  system-wide admin capability.
- **The bundle body stays schema-light in OpenAPI** (like export): the stub documents
  the query params (`ImportQuery`) + `ImportResult` + the 409, and describes the body
  in prose rather than deriving `ToSchema` across the whole nested `WorkspaceExport`.

## Capability table extension

| Change | Where |
|--------|-------|
| `POST /workspaces/import` (`token:admin`, `?mode=new\|restore`, `&force`) | `routes/workspace.rs`, `app.rs` |
| `import::flatten` + pure `import::remap` (+ unit tests) | `maidan-server/src/import.rs` |
| `WorkspaceExport: Deserialize`; `ImportQuery`/`ImportMode`/`ImportResult` DTOs | `export.rs`, `dto.rs` |
| OpenAPI stub + schemas + capability-map + matrix body clause | `openapi/`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Risks identified + still open

- **Reactions/votes aren't in the export bundle** (Cluster-187 scope), so a
  round-trip drops them. Documented; exporting them is an N+1 per message, deferred.
- **Artifact blobs aren't exported**, so an import references shas that may not exist
  in the target's artifact store (the reference rows import fine; the bytes are the
  operator's separate backup concern, per the Cluster-260 DR runbook).
- **No streaming** — the bundle is one in-memory request/response (inherited from
  export). Large-tenant streaming import/export is a follow-up.

## Forward look

Search token-aware read routing (271–272) closes the last optional deferral: give
`maidan-search`'s `PostgresSearch` its own reader pool + replay poller and honor the
`Maidan-Consistency-Token` on search reads, validated against `replica-harness.sh`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 269.0]].
