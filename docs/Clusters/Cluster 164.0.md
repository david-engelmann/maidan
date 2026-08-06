# Cluster 164.0 — channel:admin membership API (RBAC part F)

**Theme:** Make private channels *operational* — an API to grant and revoke
channel membership, not only the creator's auto-add.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v164.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `channel:admin` capability | `maidan-auth/src/capability.rs` (KNOWN; not `default_minted`) |
| REST `POST`/`GET /channels/:cid/members`, `DELETE …/:mid` | `routes/channel.rs`, `app.rs`, `dto.rs`, `openapi/paths/api.rs` |
| MCP `add`/`list`/`remove_channel_member` | `tools/channel.rs`, `tools/mod.rs`, `catalog.rs`, contracts |
| Capability maps + matrices | `contracts/*.json`, `http_capability_matrix_e2e`, `mcp_capability_matrix_e2e` |

## Why

Clusters 159–163 built + enforced channel membership everywhere, but the only
way to *add* a member was the store directly or the private-channel creator's
auto-add. Admins need a first-class API. `channel:admin` is a deliberately
granted capability (not minted by default); a holder manages membership for any
channel in their workspace.

## Non-goals

- `reference.rs` authorization — the last remaining RBAC surface (next cluster).
- Per-channel *admin* scoping (a `channel:admin` token manages all channels in
  its workspace, not a specific one) — a possible future refinement.

## Exit criteria

- Membership can be added/listed/removed via REST + MCP, gated by
  `channel:admin`; adding a member grants access end-to-end; matrices +
  OpenAPI bijection green — **met**.
- `v164.0.0` tagged.

## Verification & limits

- `channel_admin_api_manages_membership_end_to_end` (auth enabled): add member →
  private-channel access granted → list shows 2 → remove → denied again.
- HTTP + MCP capability matrices verify `channel:admin` gating (403 without it);
  OpenAPI documents the routes (map↔spec bijection preserved).
- **CI note:** GitHub Actions outage — validated locally (fmt + clippy +
  channel/capability/openapi/mcp suites); re-run CI on `main` when recovered.

## References

- [[Retros/Cluster 164.0]]; [[Clusters/Cluster 159.0]]–[[Clusters/Cluster 163.0]];
  `capability.rs`, `routes/channel.rs`, `tools/channel.rs`. Program: [[Roadmap]]
  + memory `maidan-next-arc-program`.
