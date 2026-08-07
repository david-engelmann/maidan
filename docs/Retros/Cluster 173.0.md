# Cluster 173.0 retro — structured message content

> Tag **`v173.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 3 (agentic features), part 3.

## What shipped

- A `ContentBlock` enum (`Text` / `Code` / `ToolUse` / `ToolResult` /
  `ResourceLink`, internally tagged) and `content: Option<Vec<ContentBlock>>` on
  `Message`/`NewMessage`/`EditMessage`, persisted in a new nullable column
  (Postgres JSONB, SQLite TEXT/JSON) on both backends.
- `body` stays the canonical searchable projection: `derive_body` fills it from
  the text-bearing blocks when a client posts `content` without a `body`.
- `content` on REST post/edit + MCP `post_message`/`edit_message`/
  `post_dm_message` (+ catalog schemas). Tombstone + workspace-purge null it.
- No new event kind / capability / tool-name / contract — `content` rides the
  existing `Message` payload in `MessagePosted`/`MessageEdited`.

## What was deferred

Rich `/ui` block rendering (still renders `body`); per-block editing; content in
edit-history diffs; search over structured fields; **federation/A2A
`parts↔content` propagation** (ingested messages stay body-only — logged in Open
Work); `ToolResult` nested blocks; `ArtifactRef`; `Mention` block.

## Surprises

- **The literal sprawl was the whole cost.** Adding `content` to `NewMessage`/
  `EditMessage` broke ~57 struct literals across 30 files (tests, benches, inline
  `#[cfg(test)]` modules, and a handful of production call sites). A brace-
  matching Python inserter did the bulk in one pass; the compiler swept up the
  full-`Message {…}` literals (bus/search tests) and the inline-test sites my
  scan missed (`src/*` `#[cfg(test)]`). The *schema* change was small; the
  mechanical fan-out was the work — exactly as the pre-map warned.
- **`body` being load-bearing for search made "additive" the obvious call.**
  Replacing `body` would have rippled into the generated tsvector SQL, the FTS5
  triggers, the embedding handler, and every read caller. Keeping `body` as the
  derived projection meant **zero** search/embedding changes.
- **No three-parser trap this time.** Unlike Cluster 171's event kind, no new
  `EventKind` was needed (the `Message` payload already carries content), so the
  `EventKind::parse` + store `parse_kind` sync did not apply — the pre-map
  called this out and saved a phantom task.

## Decisions

- **Additive column, `body` derived, 5 block types, internally tagged** — match
  the MCP/A2A dialect; `Mention` deferred (first-class table already exists).
- **Graceful content parse.** `row_to_message` reads content as
  `Option<Value/String>.and_then(from_*.ok())` so a malformed hand-written row
  degrades to `None` rather than failing every message read.

## Capability table extension

| Change | Where |
|--------|-------|
| Typed `ContentBlock` message content (REST + MCP, both backends), `body` derived | `maidan-types`, `maidan-store/*/messages.rs`, `routes/message.rs`, `mcp/tools/message.rs` |

## Risks identified + still open

- **Low, additive.** Nullable column; `body` always populated so search is
  provably unaffected; no contract/capability change.
- **Federation gap (flagged):** A2A-ingested / federated messages carry `body`
  only until the ingest path maps `parts → content`. In-scope to not break;
  out-of-scope to fully propagate in v1.

## Forward look

Arc 3's last item: **HITL approvals** over the elicitation transport (built in
145–148/154 — `request_client` + `elicitation/create`). Then arc 4 (token round
3). The `ToolUse`/`ToolResult` blocks shipped here are the substrate for
richer agent-tool workflows on top.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
