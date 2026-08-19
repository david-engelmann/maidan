# Cluster 243.0 retro — mute over MCP

> Tag **`v243.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 7** — Arc H.

## What shipped

- MCP `set_notification_pref` / `list_notification_prefs` — the twins of 242's REST,
  over the shared store. The mute half of Arc H is now complete over REST + MCP.

## Surprises / decisions

- **Nothing new — and that's the point.** The store method + router wiring landed in
  241/242, so 243 is pure surface: two thin handlers, the 5-place wiring, and the two
  sorted contract files. A clean, mechanical REST-then-MCP follow-on with no design
  decisions left to make.
- **`kind` as a parsed string.** MCP tools take JSON, so `kind` is a snake_case string
  `EventKind::parse`d in the handler (InvalidParams on unknown) — matching every other
  kind-taking tool, and tested with a deliberate bad-kind rejection.
- **Member-scoped, so no gate arm.** Like the inbox tools, these key on `member_id`
  and fall through the pre-dispatch channel gate; there's no channel/thread to guard.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `set_notification_pref` / `list_notification_prefs` | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None. Mute is complete over REST + MCP.

## Forward look

The **follows / subscription** half of Arc H is next: a channel-follow + thread-follow
store foundation, then the router notifying followers of activity (honoring mutes),
then its REST/MCP management. Then Arc I (email/SMTP transport, digests, presence-aware
routing, `/ui` center), then Program D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 242.0]].
