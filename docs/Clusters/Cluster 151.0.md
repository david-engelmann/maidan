# Cluster 151.0 — token-efficient lean context reads

**Theme:** First **token-efficiency** cluster (arc item B1). Stop shipping the
two full edit-body copies on every context pack by default — they were the
single largest token cost — and bound `list_messages`.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v151.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `get_thread_context` edits lean by default (`{id, message_id, editor_id, edited_at}`); opt-in `include_edits=true` restores full `body_before`/`body_after` | `crates/maidan-mcp/src/context.rs` |
| Clamp `list_messages` limit to `1..=500` (was unbounded) | `crates/maidan-mcp/src/tools/message.rs` |
| Advertise `include_edits` + the `list_messages` limit bounds in the tool schemas | `crates/maidan-mcp/src/tools/catalog.rs` |

## Why

An MCP agent packs thread context to prime a prompt. Every `MessageEdit` in the
pack carried **both** `body_before` and `body_after` — a full copy of the
message before and after each edit. For an actively-edited thread that is the
heaviest single field in the response, and `get_workspace_context` multiplies it
across every message of every thread it nests. Agents almost never need the edit
diff to reason about a thread; they need to know *that* a message was edited,
*when*, and *by whom*. So the default now carries only that metadata, and a
caller that genuinely needs the diff opts in with `include_edits=true`.

`list_messages` passed its `limit` straight through, so a negative or very large
value could pull the entire thread in one call — a token and latency footgun.

## Non-goals

- **HTTP `/threads/:id/context` parity** — that pack is a typed
  (`utoipa::ToSchema`) struct; making it lean changes the published OpenAPI
  schema. Deferred to a follow-up so this cluster stays a tight, additive MCP
  change.
- **Snippet-only search** — the other B1 candidate; semantic hits rely on the
  body, so it needs its own care. Queued.

## PR ladder (actual)

| # | Title |
|---|--------|
| 151.0.1 | `feat(mcp): lean thread-context reads — token-efficient edits` (#392) |
| 151.0.retro | `docs(retro): Cluster 151.0 + v151.0.0 tag prep` |

## Exit criteria

- Default context pack omits edit bodies; `include_edits=true` restores them;
  `list_messages` clamps; tests green — **met**.
- `v151.0.0` tagged after retro.

## Verification & limits

- Unit: `thread_context_omits_edit_bodies_by_default`,
  `thread_context_include_edits_returns_full_bodies`. Catalog/capability
  contract tests green (names/caps unchanged; only input schemas grew).
- Limit: the lean edit record is a strict subset of the full one, so a consumer
  that ignores edit bodies sees no shape change — but a consumer that *read*
  `body_before`/`body_after` from the default response must now pass
  `include_edits=true`. Documented as a **Changed** entry.

## References

- [[Retros/Cluster 151.0]]; `context.rs`, `tools/message.rs`, `tools/catalog.rs`.
