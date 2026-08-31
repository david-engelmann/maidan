# Cluster 333.0 retro — MCP edit_message emits MessageEdited (audit P1.1a)

> Tag **`v333.0.0`**. Phase XXIV (post-gate hardening). **Cluster 2 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The sharpest verified correctness bug from the audit: MCP `edit_message` called the
**event-less** `store.edit_message`, so an agent editing a message through MCP appended **no**
`MessageEdited` event. Consequences (all real, all on the primary agent transport):

- The just-shipped **as-of context replay** (`reconstruct_messages_through`) folds only the
  original `MessagePosted`, so an MCP-edited message showed its **stale body forever**.
- The embedding **indexer** subscribes to `MessageEdited` (`indexer.rs:109-111`), so an MCP
  edit **never reindexed** → stale semantic search.
- WS/SSE domain-event subscribers + the notification router never saw the edit.

Fix:
- **`McpServer::publish_stored`** — a new shared helper that bus-notifies an *already
  durably-appended* `StoredEvent` (hydrated from its payload), the MCP analogue of the REST
  `publish_stored`; no double-append (distinct from `publish_event`, which appends).
- **`edit_message`** now takes `server`, resolves `dm_conversation_id`, calls
  `edit_message_with_event` (atomic row + `MessageEdited`), and `publish_stored`s it — so an
  MCP edit behaves exactly like a REST edit across replay, reindex, and realtime.

## Surprises / decisions

- **One bus-publish fixes three symptoms.** Because the indexer, the as-of replay, the WS/SSE
  stream, and the notification router all key off the event log / bus, emitting the one
  `MessageEdited` event repairs reindex + replay + realtime together — no per-symptom code.
- **`publish_stored` is the reusable seam for the rest of P1.1.** Clusters 328/330 each had a
  local `notify`; this hoists it onto `McpServer` so the remaining event-less write tools
  (votes/reactions/pins/mention/reference — Cluster 334) migrate uniformly, and 328/330's
  local copies can fold into it.
- **Sliced edit-first.** P1.1 covers 8 event-less MCP tools; `edit_message` is the one with a
  *correctness* (not just observability) impact, so it leads. The social/reference tools +
  `post_message` `MentionRecorded` routing follow in 334.

## Test evidence

`mcp_edit_message_appends_messageedited_event` (after an MCP `edit_message`, the thread event
log contains a `MessageEdited` and `reconstruct_messages_through` returns the edited body —
neither held before the fix). Full `maidan-mcp` lib suite (57) + MCP contract-sync +
`mcp_capability_matrix_e2e` green. fmt + strict clippy + `--all-targets` + bootstrap-strip
clean.

## Forward look

**334 (P1.1b):** migrate the remaining event-less MCP write tools — `cast_vote`,
`add_reaction`, `remove_reaction`, `pin_message`, `unpin_message`, `record_mention`,
`add_reference` — to `*_with_event` + `publish_stored`, and publish `MentionRecorded` from MCP
`post_message`/`post_dm_message` (agent `@mention` notifications / `wait_for_mention`). Then
P1.2 unify the context assembler → P1.3 `whoami`/`initialize` → P1.4 post-path → P1.5 tests.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
