# Cluster 335.0 retro — MCP context: batch reads + surface artifacts (audit P1.2)

> Tag **`v335.0.0`**. Phase XXIV (post-gate hardening). **Cluster 4 of the post-flagship audit
> program.** No new gate tag.

## What shipped

The MCP context assembler re-implemented the pack independently of the REST one — with a
per-message N+1 (references + edits fetched one message at a time) and **omitting artifacts
entirely**, while the REST assembler batched both and included artifacts (Cluster 106/199).
This closes that user-visible divergence.

- **`maidan_types::artifact_shas_from_metadata`** — the sha extractor, moved out of the REST
  assembler so REST + MCP share one implementation.
- **`maidan-mcp/src/context.rs`** gained three batched helpers used by *both*
  `get_thread_context` and `get_thread_context_as_of`:
  - `collect_references` — one thread read + one `src_id = ANY` read across all messages
    (kills the N+1), ordered + deduped.
  - `collect_edit_views` — one batched edit read, re-ordered by (message pos, edited_at, id),
    with an optional as-of `cutoff`.
  - `collect_artifacts` — resolves the artifacts referenced by the page's message metadata
    (the omission fix). MCP packs now carry an `artifacts` array.

## Surprises / decisions

- **Delivered the wins, deferred the full cross-crate hoist — deliberately.** The audit's P1.2
  also envisioned hoisting one assembler into `maidan-router` to end the `as_of` double-impl.
  But `maidan-router` already exports a *different* `ThreadContext` (the resolution struct →
  name collision), and moving the pack DTOs there needs utoipa-feature propagation + a
  `futures` dep — a multi-cluster refactor whose remaining payoff is maintainability-only. The
  **user-visible** divergence (N+1 + missing artifacts) is fixed here, and the trickiest shared
  logic (the as-of message fold) already goes through `maidan_types::reconstruct_messages_through`.
  The remaining hoist is logged in Open Work with this rationale rather than forced through at
  regression risk.
- **REST behavior unchanged.** Moving the sha extractor to `maidan_types` is import-only for
  REST (the `use maidan_types::*` glob already covers it); `context_query_count_e2e` (the N+1
  regression guard) stays green, confirming REST's batching is intact.

## Test evidence

`context_pack_includes_artifacts` (an MCP pack surfaces a message-referenced artifact — it did
not before); full `maidan-mcp` lib suite (59) + the REST context regressions
(`context_query_count_e2e`, `thread_context_e2e`, `as_of_replay_e2e`, `glossary_context_e2e`,
`workspace_context_concurrency_e2e`, `context_pagination_e2e`) + types tests green. fmt +
strict clippy + `--all-targets` + bootstrap-strip clean.

## Forward look

Next: **P1.3 `whoami` + `initialize` instructions** (the cheapest adoption unlock — an agent
with only a token can't run the hero loop today) → P1.4 post-path round-trips → P1.5 egress
wire tests + LSN replica CI → P2 docs/polish.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the post-flagship audit
program ([[Open Work]]).
