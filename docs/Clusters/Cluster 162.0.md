# Cluster 162.0 — MCP aggregate-read filtering (RBAC part D)

**Theme:** Close the last MCP content leak — the aggregate reads that return a
*set* of results spanning channels (search, channel list, workspace context).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v162.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `search_messages` drops hits in inaccessible channels | `tools/search.rs` (+ store/auth params) |
| `list_channels` hides private channels the caller isn't in | `tools/channel.rs` (+ auth param) |
| `get_workspace_context` drops threads in inaccessible channels | `tools/mod.rs` (dispatch-arm filter) |

## Why

Cluster 161 gated the *point-access* MCP tools, but the aggregate reads still
returned private-channel content: `search_messages` returned hits from any
channel, `list_channels` listed private channels, and `get_workspace_context`
packed every thread. These filter a result *set* rather than a single target, so
each needed handler-level (or dispatch-arm) filtering by `can_access_channel`,
cached per channel to avoid an N+1.

## Non-goals (follow-ups, Open Work)

- WS event-subscribe gate (`subscribe_grants`), `reference.rs`, and the
  `channel:admin` membership API remain.

## PR ladder (actual)

| # | Title |
|---|--------|
| 162.0.1 | `feat(mcp): filter aggregate reads by channel access` (combined impl+retro) |

## Exit criteria

- A non-member's `search_messages` / `list_channels` / `get_workspace_context`
  exclude private-channel content; members see it; suites green — **met**.
- `v162.0.0` tagged.

## Verification & limits

- Extended `mcp_denies_non_members_in_private_channels`: Bob's `list_channels`
  omits the private channel and Alice's includes it; Bob's
  `get_workspace_context` excludes the private channel's threads. Full maidan-mcp
  (34) + server search/MCP e2e green.
- **CI note:** GitHub Actions outage — validated locally; re-run CI on `main`
  when recovered.

## References

- [[Retros/Cluster 162.0]]; [[Clusters/Cluster 161.0]]; `tools/{search,channel,mod}.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program`.
