# Cluster 194.0 — agentic: A2A ingest preserves parts as structured content

**Theme:** Arc C (agentic task-queue depth), part 5 — close the Cluster 173
deferral: A2A message ingest dropped structured content (`content: None`).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v194.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `message_content` — A2A text parts → `Vec<ContentBlock>` | `maidan-a2a/src/protocol.rs`, `lib.rs` |
| A2A ingest sets `content` from the parts (was `None`) | `maidan-server/src/a2a_agent.rs` |

## Why

Cluster 173 gave messages a typed `content` axis (blocks over `body`), on REST and
MCP. But the A2A ingress (`POST /a2a/v1/rpc` → `post_a2a_message`) built its
`NewMessage` with `content: None` — an A2A message's parts were joined into `body`
and the structure discarded, so a message's ingress determined whether it carried
structured content. Three research agents flagged the `a2a_agent.rs` `content: None`.

## The fix

`message_content(&A2aMessage)` maps each text part to a `ContentBlock::Text`
(mirroring `message_text`'s text-part filter), and the ingest sets `content` from
it. `body` stays the joined searchable projection — the two are independent
projections of the same parts (as in Cluster 173), so search/embeddings are
unchanged and a consumer reading `content` now sees the parts as blocks
regardless of whether the message arrived over REST, MCP, or A2A.

## Exit criteria

- An A2A message's text parts are preserved as structured `content` blocks; `body`
  unchanged — **met**.
- `v194.0.0` tagged.

## Verification & limits

- `maidan-a2a` `message_content_maps_text_parts_to_blocks` (only text parts become
  blocks; no text parts → `None`, mirroring `message_text`).
- `a2a_protocol_e2e::a2a_send_message_preserves_parts_as_structured_content`: a
  two-part A2A message → the stored message has `body == "part one\npart two"` and
  `content == [Text("part one"), Text("part two")]`.
- Limit: the A2A protocol subset here models only **text** parts (`TextPart`), so
  content is always `Text` blocks — there's no data/file/tool part to map to the
  richer `ContentBlock` variants yet. Federation *egress* (`parts` built from a
  Maidan message) still sends body-only; ingest is the side the sweep flagged.

## References

- [[Retros/Cluster 194.0]]; `maidan-a2a/src/protocol.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc C). Closes a [[Retros/Cluster 173.0]]
  deferral.
