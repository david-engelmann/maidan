# Cluster 269.0 retro — workspace import (store)

> Tag **`v269.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 3 — workspace import, store foundation.** No new gate tag.

## What shipped

- `WorkspaceImport` type (`maidan-types/models.rs`) — the deserializable content
  graph that mirrors the Cluster-187 `WorkspaceExport`: workspace, members, channels,
  channel_members, threads, messages, message_edits, pins, references.
- `Store::import_workspace(&WorkspaceImport)` — one transaction, all-or-nothing,
  full-column inserts that **preserve explicit ids, state, and timestamps** (so an
  exported bundle round-trips byte-faithfully). Both backends: `postgres/import.rs`
  (`$n`, JSONB `metadata`/`content` bound directly) and `sqlite/import.rs` (`?`,
  `metadata`/`content` as JSON TEXT).
- Both-backend round-trip test (`tests/workspace_import.rs`): construct a graph with
  a private channel, a closed+assigned thread, a structured-content message, a
  tombstoned message, an edit, a pin, and a reference; import; read every collection
  back and assert the private flag, thread state, assignee, and structured content
  all survived.

## Surprises / decisions

- **Zero-blast-radius store foundation (the 159/217/226 pattern).** No routes, no
  remap, no "already exists" guard — this cluster is only the raw writer. The mode
  flag (new-workspace remap vs same-id restore), the `token:admin` REST route, and
  the 409-on-conflict guard are Cluster 270's job. Landing the writer alone keeps
  the diff small and the blast radius nil (nothing calls it yet).
- **`message_edits.id` is not preserved.** It is a serial surrogate that nothing
  references; the insert omits it and lets the sequence regenerate. Every other id
  is explicit (they are FK targets and cross-reference keys).
- **Both backends in lockstep → no `backend_parity` allowlist change.** `import.rs`
  exists in both `postgres/` and `sqlite/`, so the module-parity guard passes
  untouched (unlike the Cluster-261 `replication.rs`, which was Postgres-only).

## Capability table extension

| Change | Where |
|--------|-------|
| `WorkspaceImport` bundle type | `maidan-types/src/models.rs` |
| `Store::import_workspace` (both backends) | `store.rs`, `postgres/import.rs`, `sqlite/import.rs` |

## Risks identified + still open

- **No id-collision guard yet.** Importing a bundle whose ids already exist will
  fail on the PK conflict mid-transaction (and roll back cleanly — all-or-nothing).
  The caller-facing 409 + `?mode=new` remap arrive in 270.
- **Reactions/votes are not in the export bundle** (Cluster-187 scope), so they are
  not importable/restorable — documented, unchanged here.

## Forward look

Cluster 270: `WorkspaceExport` gains `Deserialize`, flatten export→import, a pure
`remap_bundle` (fresh-id remapping for new-workspace mode), and
`POST /workspaces/import[?mode=new|restore][&force]` (`token:admin`, 409 on conflict
for restore unless `force`) with the full new-route preflight + e2e. Then search
token-aware routing (271–272).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 268.0]].
