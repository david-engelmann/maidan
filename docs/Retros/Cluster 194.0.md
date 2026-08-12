# Cluster 194.0 retro — A2A messages carry structured content now

> Tag **`v194.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 5.

## What shipped

- `message_content` maps an A2A message's text parts to `ContentBlock::Text`
  blocks, and the A2A ingest now sets `content` from it instead of `None` —
  closing a Cluster 173 deferral flagged by three research agents.

## Surprises / decisions

- **The gap was one line, but a real consistency hole.** REST and MCP posts could
  carry structured `content`; A2A silently couldn't. A message's *ingress*
  shouldn't decide whether its structure survives — a downstream consumer reading
  `content` now sees the same shape regardless of how the message arrived.
- **`body` and `content` are independent projections — keep them that way.** I
  deliberately left `body` as the existing `message_text` join (`\n`-separated)
  rather than re-deriving it from the blocks via `derive_body` (blank-line join).
  Changing `body` would shift the searchable/embedded text of every A2A message
  for no benefit; the point is to *add* structure, not reshape the projection.
- **The protocol only has text parts.** `TextPart` is the only part type in this
  A2A subset, so `content` is always `Text` blocks — the mapping can't yet produce
  `Code`/`ToolUse`/`ResourceLink`. That's a protocol limit, not a shortcut; when
  the A2A part model grows, the mapping is the one place to extend.

## Decisions

- **Mirror `message_text`'s filter** (`kind == "text"`) so `content` and `body`
  agree on which parts count — no drift between the two projections.
- **Ingest only.** Federation *egress* still builds body-only `parts`; the sweep
  flagged the ingest `content: None`, and egress is a separate, larger change
  (mapping Maidan content blocks back to A2A parts) — noted, not bundled.

## Capability table extension

| Change | Where |
|--------|-------|
| A2A ingest preserves text parts as `content` blocks | `maidan-a2a` + `a2a_agent.rs` |

## Risks identified + still open

- **Net additive, non-breaking** — `body` unchanged (search/embeddings untouched);
  `content` is additive and omitted-when-empty (Cluster 177). Open: egress
  `content→parts` (body-only today); no non-text A2A part types to map.

## Forward look

Arc C continues: structured tool-call transcripts, `wait_for_mention`, handoff
notes. Then Arc D (performance & scale).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Closes a
[[Retros/Cluster 173.0]] deferral.
