# Cluster 175.0 retro — MCP `search_messages` snippet_only parity

> Tag **`v175.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 4 (token round 3), part 1.

## What shipped

- `snippet_only` (default `false`) on the MCP `search_messages` tool: drops full
  `body` from each hit and keeps the snippet, via the same
  `SearchHit::into_snippet_only` the REST search has used since Cluster 152.
  Applied after the Cluster 162 channel-access filter. Catalog schema updated.

## Surprises

- **Pure parity, one-helper reuse.** The token-saver already existed on REST
  (152) and its logic is a `maidan-search` method with its own unit tests — so
  the MCP side was an argument + one `.map(into_snippet_only)` + a catalog
  property. No new capability/tool/contract; the largest per-result token cost
  (full bodies) is now agent-controllable on the surface agents actually use.

## Decisions

- **Opt-in, default off** — no behavior change for existing callers; one source
  of truth for the snippet semantics across REST + MCP.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `search_messages` `snippet_only` (drop bodies) | `maidan-mcp/src/tools/search.rs` |

## Risks identified + still open

- **Negligible.** Additive, opt-in, reuses a tested helper.

## Forward look

Arc 4 (token round 3) continues: capability-filtered `tools/list` + trimmed
catalog descriptions; lean write-acks / omit-empty metadata; opt-in lean event
frames.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
