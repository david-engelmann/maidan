# Cluster 152.0 — lean HTTP context pack + snippet-only search

**Theme:** Token-efficiency part 2 (arc item B1). Extend Cluster 151's MCP lean
reads to the **REST** surface, so an agent that pulls context / searches over
HTTP gets the same token savings.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v152.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ThreadContext.message_edits` → `Vec<MessageEditView>` (optional bodies); omit `body_before`/`body_after` unless `include_edits=true` | `crates/maidan-server/src/thread_context.rs` |
| `include_edits` query param on `GET /threads/:id/context` + `/workspaces/:wid/context` | `dto.rs`, `routes/thread.rs`, `routes/workspace.rs` |
| Register `MessageEditView` in the OpenAPI components | `openapi/mod.rs` |
| `snippet_only=true` on `GET /workspaces/:wid/search` — drop `body`, keep `snippet`; semantic fallback truncates `body` into `snippet` | `dto.rs`, `routes/search.rs`, `crates/maidan-search/src/hit.rs` |

## Why

Cluster 151 made the **MCP** `get_thread_context` lean by default but left the
**REST** `/threads/:id/context` pack shipping full edit bodies — an agent
integrating over HTTP saw none of the savings. The HTTP pack is a typed
(`utoipa::ToSchema`) struct, so going lean means a real (but clean) schema
change: `MessageEditView` with optional bodies.

Search had a parallel problem: each `SearchHit` carries both the full `body` and
a `snippet`. For lexical hits the `body` is redundant with the highlighted
snippet; dropping it saves the most tokens on the largest results. But
**semantic** hits arrive with an *empty* snippet and rely on `body`, so a naive
"drop body" would blank them — hence the truncated-prefix fallback.

## Non-goals

- **Default-lean search** — `snippet_only` is opt-in (default false); search
  stays fully backward compatible. (Context-pack edits *are* lean by default,
  matching 151 and flagged as a Changed entry.)
- **Unifying the two context implementations** (MCP `context.rs` vs. server
  `thread_context.rs`) — still separate; documented in the 151 retro.

## PR ladder (actual)

| # | Title |
|---|--------|
| 152.0.1 | `feat(server): lean HTTP context pack + snippet-only search` (#394) |
| 152.0.retro | `docs(retro): Cluster 152.0 + v152.0.0 tag prep` |

## Exit criteria

- HTTP context pack omits edit bodies by default; `include_edits=true` restores
  them; `snippet_only=true` drops search bodies; tests green — **met**.
- `v152.0.0` tagged after retro.

## Verification & limits

- E2E: `thread_context_edit_bodies_are_opt_in`,
  `http_search_snippet_only_drops_bodies_and_keeps_snippets`. Unit: snippet
  projection for lexical / semantic / UTF-8-boundary. OpenAPI spec +
  capability-map contract tests green with the new `MessageEditView` component.
- Limit: `snippet_only`'s semantic fallback is a plain byte-bounded prefix (no
  highlight) — agents wanting the whole message fetch it by `message_id`.

## References

- [[Retros/Cluster 152.0]]; [[Clusters/Cluster 151.0]]; `thread_context.rs`,
  `routes/search.rs`, `crates/maidan-search/src/hit.rs`.
