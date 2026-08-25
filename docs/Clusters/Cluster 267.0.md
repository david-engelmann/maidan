# Cluster 267.0 — A2A egress: content → parts

> **Optional deferrals sweep, part 1.** Phase XXIV post-gate hardening. Tag
> **`v267.0.0`**. No new gate tag.

## Goal

Close the Cluster-194 deferral: A2A *ingress* mapped `parts → content`, but the
outbound A2A message the agent returned was a raw echo of the request. Render it
instead from the **stored** message's canonical structured content.

## Scope

| Change | Where |
|--------|-------|
| `message_parts_from_content(&[ContentBlock]) -> Vec<TextPart>` (inverse of `message_content`) + unit test | `maidan-a2a/src/protocol.rs` |
| `post_a2a_message` builds the outbound A2A message from the stored content, not the echo | `maidan-server/src/a2a_agent.rs` |

## Design decisions

- **Render from stored content, don't echo.** The A2A `SendMessage` task's
  `status.message` was `Some(req.message)` — the caller's own message echoed back.
  It now comes from the *stored* `Message`'s `content` (content → parts), so an A2A
  consumer sees Maidan's canonical representation (e.g. after any normalization),
  not what it happened to send. For an A2A-ingested message the parts round-trip
  faithfully (parts → content → parts), so this is behaviour-preserving in the
  common case while being correct for content set by other paths.
- **Text-only projection, mirroring `derive_body`.** A2A parts are text-only, so
  each block maps to its text form exactly as `maidan_types::derive_body` does:
  `Text` → its text, `Code` → a fenced block, `ToolResult` → its content,
  `ResourceLink` → its title/URI. `ToolUse` has no text projection and is skipped.
  Empty result → fall back to the message body.
- **A small, tested, symmetric pair.** `message_content` (ingress) and
  `message_parts_from_content` (egress) are now a documented, round-trip-tested pair.

## Non-goals

- Federation *event* relay already carries `content` (it serializes the full
  `Message`), so no change was needed there — the gap was only the A2A agent's echo.

## Risks

- Low. The 194 ingress e2e (`a2a_send_message_preserves_parts_as_structured_content`)
  still passes; the new round-trip unit test covers the projection.
