# Cluster 243.0 — mute-preference MCP tools

> **Program C (notifications & reach), part 7** — Arc H. Phase XXIV post-gate
> hardening. Tag **`v243.0.0`**. No new gate tag.

## Goal

Give MCP-native agents the mute-preference surface REST got in 242, completing the
**mute** half of Arc H over both transports.

## Scope

| Change | Where |
|--------|-------|
| MCP `set_notification_pref` (upsert a per-kind mute) / `list_notification_prefs` — the twins of 242's REST, over the shared store | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **`kind` is a snake_case string, parsed to `EventKind`.** MCP args are JSON, so the
  tool takes `kind` as a string and `EventKind::parse`s it, returning `InvalidParams`
  on an unknown kind — the same shape as the other kind-taking MCP tools.
- **`workspace:read`, member-scoped args, no gate arm.** Setting one's own mute is
  self-config (the 242 REST cap choice), and the tools key on `member_id` (not a
  channel/thread), so they fall through the `enforce_channel_access` pre-dispatch
  gate like the other inbox/pref tools; the store method is the mutation.
- **Twins, not new logic.** REST (242) + MCP (243) call the identical
  `set_notification_pref` / `list_notification_prefs` store methods — the REST-then-MCP
  split (239 → 240, 242 → 243).

## Non-goals

- **Follows / subscription** (channel + thread follow → notify on activity) — the
  next Arc-H clusters.

## Risks

- MCP 5-place wiring + both sorted contracts; the contract-sync tests
  (`tools_catalog_contract`, `mcp_capability_map_contract`) catch a miss.
