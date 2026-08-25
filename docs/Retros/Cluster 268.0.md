# Cluster 268.0 retro — the last parity gap

> Tag **`v268.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 2.** No new gate tag.

## What shipped

- `set_member_email` / `get_member_email` / `delete_member_email` MCP tools — the
  MCP twins of the Cluster-250 REST, over the Cluster-248 store. Standard 5-place
  wiring + both sorted contracts; inline `email_tools_set_get_delete` e2e.

## Surprises / decisions

- **Pure parity, no new logic.** The store methods and the validation rule (light
  `@` check, full validation at the transport) already existed; this just exposes
  them over MCP with the same member-scoped `workspace:read` shape as the
  delivery-mode and notification-pref tools. The contract-sync tests are the
  guardrail that the 5 places stayed in lockstep.
- **`get` returns `null`, not an error, when unset.** MCP `content_json(&Option<..>)`
  serializes `None` to `null` — a cleaner "no address" signal for an agent than an
  error, matching how the other read tools surface absence.

## Capability table extension

| Change | Where |
|--------|-------|
| `set/get/delete_member_email` MCP tools | `tools/member.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None — a member-scoped read/write trio over existing store methods.

## Forward look

Remaining optional deferrals: workspace import — both modes (269–270), search
token-aware routing (271–272).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 267.0]].
