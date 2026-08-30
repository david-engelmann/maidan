# Cluster 330.0 retro — context snapshot MCP tool

> Tag **`v330.0.0`**. Phase XXIV (post-gate hardening). **Cluster 12 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The MCP twin of Cluster 329: an agent can freeze a context snapshot without leaving the
tool surface.

- **MCP `snapshot_thread_context`** (`tools/snapshot.rs`) — same params as
  `get_thread_context` (`thread_id` + `message_limit`/`transition_limit`/`include_edits`/
  `include_glossary`/`as_of`); builds the pack via the shared `context::get_thread_context`
  builder, serializes it, `artifacts.put`s the bytes, and `upsert_artifact_with_event`s the
  row + the Cluster-204 per-workspace ref + `ArtifactUpserted` event, then bus-notifies the
  returned event. Returns the `Artifact` (`kind=context_snapshot`). Standard 5-place wiring
  (`artifact:upload`; `thread_id` pre-dispatch gate; both sorted contracts → 84 tools).

## Surprises / decisions

- **Used the modern atomic path, not the old MCP artifact convention.** The existing MCP
  artifact tools call `store.upsert_artifact` (no event, **no Cluster-204 ref** — so their
  blobs 404 for a non-bypass caller). The snapshot tool uses `upsert_artifact_with_event`
  with `ref_workspace`, so an MCP-frozen snapshot is fetchable by its workspace — more
  correct than the sibling tools (a pre-existing MCP gap noted for a future sweep).
- **Reused `context::get_thread_context`** — the same builder the `get_thread_context` tool
  returns raw; the snapshot serializes that exact Value, so dedup within the MCP surface is
  deterministic.
- Bus-notify hydrates the returned `StoredEvent` (the same `notify` shape as Cluster 328's
  seed tool) — atomic durable log + real-time parity.

## Test evidence

`snapshot_thread_context_tool_freezes_and_refs` (real-member session: `context_snapshot`
artifact returned, the Cluster-204 ref recorded for the workspace, an identical snapshot
dedups to the same sha); MCP contract-sync (84 tools) + `mcp_capability_matrix_e2e`
(deny-without-`artifact:upload` + pass-with) green. fmt + strict clippy + `--all-targets` +
bootstrap-strip clean; mdbook linkcheck green.

## Forward look

Context snapshot is now complete over REST + MCP. The flagship arc's remaining tail is all
**optional convenience** over existing primitives — seed `pack`/`prefix` inclusion (compose
snapshot + seed, or as-of replay), a `WorkSeeded` single-signal event (covered by
`ThreadCreated` + `ReferenceAdded`), and item 7 flow/setup template (covered by
export/import). **Cluster 331 will close the arc** with an explicit decision on that tail,
reaching a clean point to open a research round.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
