# Cluster 163.0 — verified WS/MCP subscribe grants (RBAC part E)

**Theme:** Close the private-channel **event** leak — the subscribe path trusted
the client's asserted `channel_grants`, so a non-member could receive a private
channel's events.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v163.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Verify asserted private grants against `channel_is_member`; drop unverified | `subscribe_grants.rs` (+ `auth` param) |
| Resolve caller identity *before* applying grants | `ws.rs` |
| Pass `AuthContext` through | `mcp_stream.rs` |

## Why

`apply_subscribe_grants` added a client-asserted private channel to the allowed
set without checking membership, then denied only the private channels *not*
asserted. So a non-member subscribing with `channel_grants: [private_id]`
received that channel's message/reaction/pin events over WS or MCP SSE — a
content leak parallel to the REST/MCP read gaps closed in 160–162. Now each
asserted private-channel grant is verified against `channel_is_member`; an
unverified grant is dropped, so the channel is denied and added to
`private_channel_deny`.

The WS handler previously resolved auth *after* applying grants; it now resolves
the caller (bearer/session, or `bypass` when auth is disabled) first and passes
it in. MCP SSE already had the `AuthContext`.

## Non-goals (Open Work)

- `reference.rs` authorization and the `channel:admin` membership API remain.

## Exit criteria

- A non-member's asserted private grant is dropped and the channel denied; a
  member's is honored; existing subscribe/ws tests stay green — **met**.
- `v163.0.0` tagged.

## Verification & limits

- Unit `asserted_private_grant_is_dropped_for_non_members` (auth enabled): the
  grant is dropped + denied for a non-member, honored after they join. The 16
  existing subscribe/ws/mcp-stream e2e tests (which run bypass or with real
  members) stay green.
- **CI note:** GitHub Actions outage — validated locally; re-run CI on `main`
  when recovered.

## References

- [[Retros/Cluster 163.0]]; [[Clusters/Cluster 162.0]]; `subscribe_grants.rs`,
  `ws.rs`, `mcp_stream.rs`. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
