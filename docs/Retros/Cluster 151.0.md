# Cluster 151.0 retro — token-efficient lean context reads

> Tag **`v151.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> First token-efficiency cluster (arc item B1).

## What shipped

- **`get_thread_context` edits are lean by default.** Each edit record now
  serializes as `{id, message_id, editor_id, edited_at}` — the
  was-edited/when/by-whom signal — instead of the full `body_before` +
  `body_after` copies. New opt-in **`include_edits: true`** restores the full
  bodies. `get_workspace_context` inherits the lean default through its nested
  per-thread packs (its biggest multiplier: N threads × edits).
- **`list_messages` limit clamped to `1..=500`** (was passed straight through —
  a negative or huge value could pull the whole thread).
- **Catalog schemas** advertise `include_edits` (bool, default false) and the
  `list_messages` limit bounds, so the constraint is discoverable.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Follow-up | HTTP `/threads/:id/context` lean parity | It's a typed `utoipa::ToSchema` pack; going lean changes the published OpenAPI schema — bigger than this additive MCP change. |
| Queued | Snippet-only search projection | The other B1 candidate; semantic hits rely on the body, so it needs its own handling. |
| n/a | The N+1 edit fetch in `context.rs` | Still one `list_message_edits` per message (the HTTP pack already batches). Out of scope — token weight, not query count, was the target. |

## Surprises

- **Two context implementations, not one.** The MCP tool
  (`maidan-mcp/src/context.rs`, N+1 edit fetch, no artifacts) and the HTTP
  handler (`maidan-server/src/thread_context.rs`, batched, typed, with
  artifacts) are separate code paths that both build "thread context." They had
  already diverged before this cluster; the lean default widens the gap
  (documented, parity queued) rather than unifying them under time pressure.

## Decisions

- **Lean-by-default, opt-in-full** over opt-out. A token-efficiency change whose
  win only lands if the caller remembers to ask isn't much of a win — the
  default is where the tokens are spent. The cost is a **behavior change**
  (edit diffs vanish from the default response); mitigated because the lean
  record is a strict subset of the full shape and the diff is one flag away.
- **Keep the metadata, drop the bodies** rather than omit edits entirely. An
  agent still learns a message was edited (and when/by whom) for near-zero
  tokens; only the rarely-needed diff text is gated.

## Capability table extension

| Capability | Where |
|------------|-------|
| Lean `get_thread_context` edits + `include_edits` opt-in | `crates/maidan-mcp/src/context.rs` |
| `list_messages` limit `1..=500` | `crates/maidan-mcp/src/tools/message.rs` |

## Risks identified + still open

- **Low, but a real behavior change.** A caller that read
  `body_before`/`body_after` from the default `get_thread_context` /
  `get_workspace_context` response now gets metadata only and must pass
  `include_edits=true`. Called out in CHANGELOG under **Changed**.

## Forward look

First of the token-efficiency arc. Natural next steps: **HTTP context-pack
parity** (same lean default behind the typed schema), **snippet-only search**
(drop the redundant body on lexical hits), and clamping/`include_*` knobs on the
remaining broad reads. The live-updating `/ui` thread view and the
`request_client` GET-stream fix remain queued from the same next-arc research.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
