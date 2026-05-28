# Cluster 29.0 retro — Message edit

> Closing wave for Cluster 29.0 · target tag **`v29.0.0`**.

Slack-style message edit: update body/metadata, set `edited_at`, fan-out on bus and search.

## What shipped

- `EditMessage`, `Event::MessageEdited`, `Store::edit_message` (Postgres + SQLite).
- `PATCH /messages/:id` — author `message:post`, others `workspace:write`; rejects tombstoned.
- MCP `edit_message` tool + resource subscription fan-out.
- Search indexer + `EmbeddingHandler` on `MessageEdited`.
- Federation `remap_event_workspace` for edits.
- Tests: `message_edit`, `message_edit_postgres`, `http_crud_e2e`, `mcp_e2e`.

## What was deferred

| To | What | Why |
|----|------|-----|
| Post-29 | Edit history / diff audit | API-first; no version table |
| Post-29 | UI edit affordance | `/ui` still read-oriented |
| Post-29 | MCP bus publish on tool-only edit | Same gap as `post_message` via MCP |

## Surprises

- Axum `.patch()` on a merged route does not need `routing::patch` import.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| HTTP message edit | `v29.0.0` |
| MCP `edit_message` | `v29.0.0` |
| `MessageEdited` bus event | `v29.0.0` |

## Forward look

Next from [[Remaining Work]]: HTTP rate limits, MCP bidirectional streamable session, Helm umbrella, or artifact purge in workspace erasure.

## Acknowledgements

- Maintainer-driven cluster 29 implementation.
