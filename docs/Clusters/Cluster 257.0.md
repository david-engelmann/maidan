# Cluster 257.0 — delivery-mode MCP tools

> **Program C (notifications & reach), part 21** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v257.0.0`**. No new gate tag.

## Goal

The MCP twin of the Cluster-256 delivery-mode REST, so an MCP-only agent can read
and set a member's email delivery mode. This closes the core of Arc I — the digest
feature is now reachable over both surfaces.

## Scope

| Change | Where |
|--------|-------|
| `set_delivery_mode` / `get_delivery_mode` MCP tools | `tools/member.rs` |
| 5-place wiring: dispatch + capability arms, catalog schemas, both contract JSONs | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| Inline e2e | `server.rs` |

## Design decisions

- **Member-scoped, `workspace:read`, no gate arm.** Both tools take an explicit
  `member_id` and fall through the pre-dispatch channel/thread access gate (there's
  no channel/thread to gate) — the same shape as the Cluster-243 notification-pref
  tools. The capability is `workspace:read` (matching the REST), and the
  bearer-orchestrator model applies (no session self-only check in MCP).
- **`mode` is a snake_case string parsed to `EmailDeliveryMode`.** An unknown mode
  is `InvalidParams` — mirroring the `set_notification_pref` `kind` handling rather
  than the REST's serde-at-the-extractor path (MCP tool args are a loose `Value`).
- **Both return `{mode}`.** `set` echoes the mode it stored; `get` returns the
  current mode (`immediate` by default). A minimal, symmetric shape.

## Non-goals / deferred

- **Optional MCP email tools** (set/clear the delivery address) — low value (email is
  human-facing config); still deferred.

## Risks

- Contract sync — covered by `mcp_capability_map_contract` (map ↔ dispatch) +
  `tools_catalog_contract` (catalog ↔ tool-names), both of which enforce the sorted
  JSONs; behaviour by the inline `delivery_mode_tools_get_and_set` test.
