# Cluster 246.0 — follows MCP tools (closes Arc H)

> **Program C (notifications & reach), part 10** — **closes Arc H**. Phase XXIV
> post-gate hardening. Tag **`v246.0.0`**. No new gate tag.

## Goal

Give MCP-native agents the follow surface REST got in 245, completing Arc H
(preferences + subscription) over both transports.

## Scope

| Change | Where |
|--------|-------|
| MCP `follow_channel` / `unfollow_channel` / `list_channel_follows` + the thread triple — the twins of 245's REST | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **`follow_*` gates on target access; `unfollow_*` / `list_*` don't.** Following a
  channel/thread means subscribing to its activity, so `follow_channel` /
  `follow_thread` join the existing `channel_id` / `thread_id` pre-dispatch access
  gate arms (you can't subscribe to what you can't read). Unfollowing and listing your
  own follows need no access check — you can unfollow a channel you've since lost
  access to — so they stay out of the gate.
- **Member-scoped args, `workspace:read`.** All six take `member_id` (+ the target id
  for follow/unfollow), matching the 245 REST cap; a bearer follows on behalf of any
  member (orchestrator model).
- **Twins, not new logic.** REST (245) + MCP (246) call the identical
  `follow_channel` / `unfollow_channel` / `list_channel_follows` store methods — the
  REST-then-MCP split.

## Non-goals

- Nothing deferred within Arc H — this closes it. Arc I (email/SMTP transport,
  digests, presence-aware routing, `/ui` notification center) is next.

## Risks

- MCP 5-place wiring × 6 tools + both sorted contracts; the contract-sync tests catch
  a miss. The two `follow_*` tools are added to the pre-dispatch gate arms (channel /
  thread), so a caller can't follow a private channel/thread they lack access to.
