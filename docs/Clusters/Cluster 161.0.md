# Cluster 161.0 — private-channel access control over MCP (RBAC part C)

**Theme:** Extend the RBAC enforcement flip from REST (160) to the MCP tool
surface — an MCP agent must not read or write a private channel it isn't in.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v161.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Pre-dispatch per-channel gate for point-access content tools | `crates/maidan-mcp/src/tools/mod.rs` (`enforce_channel_access`, called at the top of `dispatch`) |
| Gate `resources/read` for `threads/{id}` + `channels/{id}` | `crates/maidan-mcp/src/server.rs` |
| MCP enforcement test | `server.rs` (`mcp_denies_non_members_in_private_channels`) |

## Why

Cluster 160 closed the REST surface, but MCP tools bypassed it — the MCP content
handlers don't receive `auth`, so an MCP agent could still read a private
thread's messages/context, post to it, or react/pin/vote/mention. Rather than
thread `auth` into every handler, the gate lives once in `dispatch`: it reads the
tool's id argument (`channel_id`/`thread_id`/`message_id`) and calls the matching
`ensure_*` helper before the handler runs. `resources/read` gets the same
treatment for the two content-bearing resource URIs.

## Semantics

Bypass callers pass through. DM tools rely on their own participant checks (the
`__dm__` channel is exempt in `ensure_*`). Public channels are unaffected.

## Non-goals (follow-ups, tracked in Open Work)

- **MCP aggregate reads** — `search_messages`, `get_workspace_context`,
  `list_channels` still return private content; they filter *result sets* (not a
  single target) so they need handler-level changes — the next cluster.
- WS event-subscribe gate, `reference.rs`, DM-via-generic-route, and the
  `channel:admin` membership API remain.

## Exit criteria

- A non-member MCP call into a private channel is denied; a member is allowed;
  the MCP capability-matrix + streamable + stream suites stay green — **met**.
- `v161.0.0` tagged.

## Verification & limits

- `mcp_denies_non_members_in_private_channels` (non-bypass `AuthContext::from_session`):
  non-member `list_messages` → `Forbidden`; member → ok. Full maidan-mcp suite
  (34) + server MCP e2e green.
- **CI note:** shipped during the GitHub Actions outage — validated locally
  (fmt + clippy + maidan-mcp + server MCP e2e); re-run CI on `main` when GitHub
  recovers.

## References

- [[Retros/Cluster 161.0]]; scratchpad `rbac-plan.md`; [[Clusters/Cluster 160.0]];
  `tools/mod.rs`, `server.rs`. Program: [[Roadmap]] + memory `maidan-next-arc-program`.
