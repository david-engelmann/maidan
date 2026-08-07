# Cluster 173.0 — agentic: structured message content

**Theme:** Arc 3 (agentic features), part 3 — typed content blocks on messages,
the agent-native content model.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v173.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ContentBlock` enum + `content: Option<Vec<ContentBlock>>` on `Message`/`NewMessage`/`EditMessage` + `derive_body` | `maidan-types/src/models.rs` |
| Migration: `content` column on `maidan_messages` (pg `0034` JSONB / sqlite `0033` TEXT) | `migrations/*/00xx_message_content.sql` |
| Store: persist + read `content` (both backends); tombstone/purge null it | `maidan-store/src/{postgres,sqlite}/messages.rs`, `purge_workspace.rs` |
| REST + MCP: `content` on post/edit; body derived when omitted | `routes/message.rs`, `dto.rs`, `mcp/tools/message.rs`, `catalog.rs` |
| OpenAPI: register `ContentBlock` | `openapi/mod.rs` |

## Why

A `Message` was `body: String` + `metadata: Value`. Agent-to-agent work wants
**typed content** — an ordered list of blocks (text, code, tool-call,
tool-result, resource link) — instead of cramming everything into a string +
ad-hoc metadata. This is the MCP/Anthropic content-block model.

## Key decisions (from the design pre-map)

- **Additive, `body` stays canonical.** `content` is added *alongside* `body`;
  `body` remains the plain-text projection that full-text + semantic search
  index. When a client posts `content` without a `body`, the server derives
  `body` from the text-bearing blocks (`derive_body`). So **search, embeddings,
  `SearchHit`, and every existing read are unchanged** — no search migration.
- **5 block types (v1):** `Text`, `Code`, `ToolUse`, `ToolResult`,
  `ResourceLink`; internally tagged (`{"type":"text",…}`) to match MCP/A2A.
  `Mention` deferred (mentions have a first-class table already).
- **New nullable column**, not folded into `metadata` (keeps typing + the
  `metadata->>'topic'` search weighting clean). `NULL` = plain/legacy message.
- **No new event kind / capability / tool / contract.** `MessagePosted`/
  `MessageEdited` already embed the full `Message`, so `content` rides along;
  the change is invisible to the event-kind + capability contracts.

## Non-goals (deferred)

Rich `/ui` block rendering (still renders `body`); per-block editing; content in
edit-history diffs; search over structured fields; **federation/A2A `parts↔content`
propagation** (ingested/federated messages stay body-only for now — flagged in
Open Work); `ToolResult` nested blocks; `ArtifactRef`; `Mention` block.

## Exit criteria

- Post/edit accept typed content over REST + MCP; body is derived; content
  round-trips both backends; search unaffected; suites green — **met**.
- `v173.0.0` tagged.

## Verification & limits

- `message_content_e2e` (REST, sqlite): content post derives body (text + fenced
  code; `tool_use` name does **not** leak into body) + round-trips via GET;
  plain body-only post → `content` null; editing content re-derives body.
- `postgres_message_content_round_trips_via_jsonb` (store, pg): post/get/list
  round-trip through JSONB; tombstone nulls content.
- Limit: a content-only message whose blocks carry no prose (e.g. a lone
  `ToolUse`) has a short/empty `body` and is weakly searchable — acceptable;
  searching structured fields is deferred.

## References

- [[Retros/Cluster 173.0]]; `maidan-types/src/models.rs`,
  `maidan-store/src/*/messages.rs`, `routes/message.rs`, `mcp/tools/message.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program`.
