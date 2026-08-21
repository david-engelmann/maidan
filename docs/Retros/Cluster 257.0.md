# Cluster 257.0 retro — the mode over MCP

> Tag **`v257.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 21** — Arc I.

## What shipped

- `set_delivery_mode` / `get_delivery_mode` MCP tools (`workspace:read`,
  member-scoped) — the twins of the Cluster-256 REST. An MCP-only agent can now read
  and switch a member between immediate emails and a digest. Standard 5-place wiring
  + both sorted contract JSONs; inline e2e `delivery_mode_tools_get_and_set`.

## Surprises / decisions

- **No gate arm — the tools fall through.** `member_id` is an explicit arg and there's
  no channel/thread to access-check, so both tools sit in the plain `workspace:read`
  capability group and skip the pre-dispatch `enforce_channel_access` gate — exactly
  the notification-pref / inbox tool shape. (The self-only guard the REST applies is
  a session concept; MCP is bearer-orchestrator, so it doesn't apply.)
- **String-parse `mode`, not serde-at-the-extractor.** REST got `400` for free by
  wrapping the enum in the request DTO; MCP tool args are a loose `Value`, so the
  handler parses the snake_case `mode` and returns `InvalidParams` on an unknown one
  — the same pattern as `set_notification_pref`'s `kind`.
- **Nothing new to learn on the wiring.** Adding an MCP tool is a well-worn 5-place
  drill by now (handler + dispatch + capability + catalog + both sorted contracts);
  the contract-sync tests catch a missed spot immediately. The only care is keeping
  the JSONs sorted (`get_delivery_mode` before `get_dependency_results`;
  `set_delivery_mode` before `set_notification_pref`).

## Capability table extension

| Change | Where |
|--------|-------|
| `set_delivery_mode` / `get_delivery_mode` MCP tools | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None — a member-scoped read/write pair over an existing store method.

## Forward look

Arc I's core is complete: email/SMTP transport (247) → address store (248) → router
wiring (249) → address REST (250) → `/ui` center (251) → presence-aware routing
(252–253) → digests (254–257). The only remaining Arc-I item is the optional,
low-value MCP email-address tools. **Program D (scale & durability)** is next.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 256.0]].
