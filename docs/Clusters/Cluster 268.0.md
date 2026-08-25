# Cluster 268.0 — MCP email-address tools

> **Optional deferrals sweep, part 2.** Phase XXIV post-gate hardening. Tag
> **`v268.0.0`**. No new gate tag.

## Goal

MCP parity for the member delivery-email surface: the REST endpoints shipped in
Cluster 250, but an MCP-only agent had no way to set/read/clear a member's address.

## Scope

| Change | Where |
|--------|-------|
| `set_member_email` / `get_member_email` / `delete_member_email` MCP tools | `tools/member.rs` |
| 5-place wiring (dispatch + capability arm, catalog, both contract JSONs) | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| Inline e2e | `server.rs` |

## Design decisions

- **Direct parity with the 250 REST over the 248 store.** `set` does the same light
  `@`/length sanity check (→ `InvalidParams`), `get` returns the `MemberEmail` or
  `null`, `delete` returns `{deleted}`. All `workspace:read`, member-scoped
  (`member_id` arg → no pre-dispatch gate arm, the notification-pref / delivery-mode
  tool shape).
- **No new store work.** The Cluster-248 `set/get/delete_member_email` methods are
  reused verbatim; this is purely the MCP surface.

## Non-goals

- Address verification (a confirm-your-email flow) — still deferred, as noted at 248.

## Risks

- Low. Inline `email_tools_set_get_delete` e2e (unset→null, set→get, bad-address
  rejected, delete→null) + the `mcp_capability_map_contract` / `tools_catalog_contract`
  sync tests.
