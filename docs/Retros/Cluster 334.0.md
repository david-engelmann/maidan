# Cluster 334.0 retro — MCP write-path events, the rest (audit P1.1b)

> Tag **`v334.0.0`**. Phase XXIV (post-gate hardening). **Cluster 3 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The rest of the MCP write-path parity (P1.1b), finishing what Cluster 333 (`edit_message`)
began. Seven MCP write tools were event-less — they mutated the store but appended no domain
event, so they were invisible to WS/SSE, at-least-once delivery, federation, and (for
mentions) the notification router. All now emit, via `McpServer::publish_stored` (the seam
added in 333).

- **`cast_vote` / `add_reaction` / `remove_reaction` / `pin_message` / `unpin_message`**
  (`tools/social.rs`) and **`add_reference`** (`tools/reference.rs`) → migrated to the
  `*_with_event` store methods + `publish_stored`. The conditional ones (`remove_reaction`,
  `unpin_message`) publish only when a row actually changed (`(bool, Option<StoredEvent>)`).
- **`record_mention`** (the explicit-mention API) → `record_mention_with_event` + publish.
- **`post_message` / `post_dm_message`** now publish a `MentionRecorded` event per @mentioned
  member (a new shared `publish_routed_mentions` helper — the MCP analogue of the REST one),
  so agent-to-agent `@mentions` fire the notification router / `wait_for_mention`. Previously
  MCP posts *recorded* the mention rows but never *published* the events.

## Surprises / decisions

- **Two event mechanisms, matched to REST.** Domain mutations with a row (votes/reactions/
  pins/references/mention-API) use `*_with_event` + `publish_stored` (atomic append). Auto-
  parsed `@mentions` on a post have no dedicated row of their own, so they use `publish_event`
  (append standalone `MentionRecorded` + bus) — exactly the split REST makes (`*_with_event`
  vs `publish`).
- **`publish_event` only logs when a bus is attached; `*_with_event` always logs.** So the
  test attaches an `InMemoryBus` to prove `MentionRecorded`/`MessagePosted` reach the log; the
  `*_with_event` events (VoteCast) log regardless. Mention *recording* (the inbox rows) still
  happens unconditionally.
- **`ReferenceAdded` isn't workspace-hoisted**, so it doesn't surface in a workspace- or
  thread-scoped event query (references span entities). Left as-is — `add_reference` uses the
  identical `*_with_event` path as `cast_vote` (asserted), and it's a pre-existing store detail,
  not a 334 regression.

## Test evidence

`mcp_write_tools_emit_domain_events` (a bus-attached server: MCP `cast_vote` → `VoteCast` in
the log; MCP `post_message` with `@bob` → `MentionRecorded` + `bob`'s mention row). The full
`maidan-mcp` lib suite (59) — including every existing vote/reaction/pin/reference tool test,
which still pass through `call_tool` — plus MCP contract-sync + `mcp_capability_matrix_e2e`
green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

**P1.1 (MCP write-path parity) is complete** (333 edit + 334 the rest). Next: **P1.2 unify the
REST↔MCP context assembler** (the MCP one has an N+1 + omits artifacts) → P1.3 `whoami` +
`initialize` instructions → P1.4 post-path round-trips → P1.5 egress wire tests + LSN replica CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
