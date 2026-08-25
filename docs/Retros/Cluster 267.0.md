# Cluster 267.0 retro — the mirror was mostly already there

> Tag **`v267.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 1.** No new gate tag.

## What shipped

- `message_parts_from_content` (the egress inverse of Cluster-194's `message_content`)
  and a change so the A2A agent's outbound message renders from the *stored* message's
  content instead of echoing the request. Closes the "federation egress content→parts"
  deferral.

## Surprises / decisions

- **Recon shrank the deferral.** The Open Work item read as "federation egress is
  body-only," but on inspection federation relays full serialized events — the
  `Message` (with its `content` field) is preserved as JSON, no down-conversion. The
  only real gap was cosmetic: the A2A `SendMessage` response echoed the *inbound*
  message rather than rendering Maidan's stored representation. So the cluster is a
  small, correct fix, not the larger conversion the label implied — and I resisted
  adding an unused conversion helper just to satisfy the wording.
- **Render-from-stored is the right default, and it's behaviour-preserving.** For an
  A2A-ingested message the parts round-trip (parts → content → parts), so the
  response is identical in the common case; it only *differs* when the stored content
  was set/normalized by another path, which is exactly when reflecting canonical
  state is more correct.
- **Mirror `derive_body` for the projection.** Rather than invent a new per-block
  text rendering, `content_block_text` matches `derive_body`'s exactly, so the A2A
  parts and the searchable `body` stay consistent.

## Capability table extension

| Change | Where |
|--------|-------|
| `message_parts_from_content` + A2A egress render | `maidan-a2a/src/protocol.rs`, `maidan-server/src/a2a_agent.rs` |

## Risks identified + still open

- Low; ingress e2e green, round-trip unit test added.

## Forward look

Remaining optional deferrals: MCP email-address tools (268), workspace import — both
modes (269–270), search token-aware routing (271–272).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 266.0]].
