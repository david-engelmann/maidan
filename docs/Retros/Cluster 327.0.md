# Cluster 327.0 retro — seed-from-message (REST)

> Tag **`v327.0.0`**. Phase XXIV (post-gate hardening). **Cluster 9 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The write side of "re-ask": `POST /messages/:id/seed` spawns a **titled, claimable child
thread** from a source message, with lineage recorded as a typed reference edge — the source
untouched, N seeds per source.

- **`RelationKind::SeededFrom`** — the lineage relation, added to the controlled vocabulary
  (`CONTROLLED` is now 8: `…/supersedes/seeded_from`).
- **`POST /messages/:id/seed`** (`SeedFromMessage { title, inclusion?, channel_id? }`) —
  creates the child thread (in the source's channel or `channel_id`) and a `seeded_from`
  reference (**new thread → source message**), each via its `*_with_event` path (atomic row +
  `ThreadCreated` / `ReferenceAdded` events; `publish_stored` notifies the bus). `inclusion`:
  `pointer` (default, edge only) or `quote` (a first message quoting the source, authored by
  the caller). Gated `workspace:write` + read-access to the source + write-access to the
  target channel.
- Reuses existing primitives — **no bespoke lineage table, no new event kind.** Lineage is
  observable as `GET /references?dst_kind=message&dst_id=…&relation=seeded_from` (the
  Cluster-320 reverse query); a "what did this spawn" walk is the forward query.

## Surprises / decisions

- **No `WorkSeeded` event (yet).** The plan named one, but the seed already emits
  `ThreadCreated` + `ReferenceAdded`, which together *are* the "a branch spawned from message
  X" signal. A dedicated `WorkSeeded` (one event a re-ask watcher can filter on without
  correlating two) is a deferred convenience, not new information.
- **`pointer` + `quote` only; `pack`/`prefix` deferred.** `prefix` (start the seed with the
  source thread's history up to the source) delegates to Cluster-326 as-of replay and is its
  own slice — a claimer can already fetch `…/context?as_of=<source's posting event>`.
- **Compound action under one cap.** The seed creates a thread + a reference (+ maybe a
  message) as a single `workspace:write` gesture; the quote message rides that authorization
  rather than a separate `message:post` check — the seed is one authorized "branch work" op.
- **MCP tool split to 328** (the 219/220, 228/229 REST-then-MCP precedent) to keep this
  cluster reviewable.

## Test evidence

`seed_from_message_e2e` (auth-enabled: pointer child + seeded_from edge via the reverse
query; quote child whose first message quotes the source; N seeds → two edges; source thread
untouched; bad-inclusion + empty-title → `400`); `relation_kind` types tests (`SeededFrom`
round-trip, `CONTROLLED.len() == 8`); `openapi_e2e` bijection + `http_capability_matrix_e2e`
(new `/seed` route denied without `workspace:write`, `{title}` body clause) +
`references_reverse_e2e` green. fmt + strict clippy + `--all-targets` + bootstrap-strip
clean; mdbook linkcheck green.

## Forward look

**328**: the MCP `seed_from_message` tool (twin of this route). Then the last arc items —
immutable context snapshot artifact → flow template — and optionally `pack`/`prefix`
inclusion + a `WorkSeeded` signal.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
