# Cluster 29.0 — Message edit

**Theme:** Slack-style message edit — update body/metadata, set `edited_at`, fan-out on bus and search.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Types | `EditMessage`, `Event::MessageEdited`, `EventKind::MessageEdited` |
| Store | `Store::edit_message` (Postgres + SQLite); reject tombstoned |
| HTTP | `PATCH /messages/:id` — author needs `message:post`, others `workspace:write` |
| MCP | `edit_message` tool + resource subscription fan-out |
| Search | Indexer + `EmbeddingHandler` subscribe to `MessageEdited` |
| Federation | `remap_event_workspace` for `MessageEdited` |

## Out of scope

- UI edit affordance (deferred)
- MCP bus publish on tool-only edits (same gap as `post_message` via MCP)
- Edit history / audit diff

## PR

Single PR: `feat/cluster-29-message-edit` → tag `v29.0.0` after retro.

## Tests

- `maidan-store/tests/message_edit.rs`, `message_edit_postgres.rs`
- `maidan-server/tests/http_crud_e2e.rs` (PATCH)
- `maidan-server/tests/mcp_e2e.rs` (`edit_message` tool)
