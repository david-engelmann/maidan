# Cluster 246.0 retro — follows over MCP (Arc H closes)

> Tag **`v246.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 10** — **closes Arc H**.

## What shipped

- MCP `follow_channel` / `unfollow_channel` / `list_channel_follows` + the thread
  triple — the twins of 245's REST, over the shared store. **Arc H (preferences +
  subscription) is complete over REST + MCP.**

## Surprises / decisions

- **Selective gating, reusing the existing arms.** `follow_channel` / `follow_thread`
  slot straight into the pre-dispatch `channel_id` / `thread_id` access-gate arms —
  the same gate every other channel/thread-scoped tool uses — so a caller can't
  subscribe to a private target they can't read, for free. `unfollow_*` and `list_*`
  are deliberately *not* in the gate: unfollowing a channel you've lost access to
  should still work, and listing your own follows takes no target id.
- **Six mechanical tools, one store surface.** Nothing new below the tool layer — 245
  already built the store + router. This is pure MCP surface: six thin handlers, the
  5-place wiring, and the two sorted contract files. The contract-sync tests are the
  net for the six sorted-JSON edits.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `follow_channel` / `unfollow_channel` / `list_channel_follows` + thread triple | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None. Preferences + subscription (Arc H) are complete over REST + MCP.

## Forward look

**Arc H is complete** — mute preferences (241–243) + follows/subscription (244–246),
the routing brain the notification router (238) consults. **Arc I** is next: an
email/SMTP transport (off-platform reach — adds an email dependency, so a cargo-deny
licence/advisory pass), scheduled digests + unread rollups, presence-aware routing
(needs durable `last_seen` — presence is in-memory today), and a `/ui` notification
center. Then **Program D (scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 245.0]].
