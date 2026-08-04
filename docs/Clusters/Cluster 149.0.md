# Cluster 149.0 — MCP inbox + mention tools

**Theme:** First of the **MCP-agent-surface arc (149–150)** — close the gap
where an MCP-only agent could be @mentioned but had no MCP way to discover it.
Surfaced by the next-arc research (missing-features thread).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v149.0.0`**, no new gate tag.

---

## Scope

| Tool | Backs | Capability |
|------|-------|-----------|
| `list_mentions` | `list_mentions_for_member` | `workspace:read` |
| `get_inbox` | `list_member_inbox` | `workspace:read` |
| `mark_inbox_read` | `advance_inbox_last_read_at` → returns inbox | `workspace:read` |

All mirror the existing HTTP handlers (`routes/member.rs`); limits clamp to
(1, 500).

## Why

The store + HTTP have had inbox/mention reads since the inbox-cursor work, but
they were never in the MCP catalog. So an MCP-only agent — the primary
consumer — could receive an @mention (`record_mention` *is* an MCP tool) yet
have no tool to find out. This is the first slice of making MCP-only agents
first-class collaborators.

## Wiring (the per-tool checklist)

`tools/member.rs` (handlers) → `tools/mod.rs` (`mod member`, dispatch +
`required_capability` arms) → `catalog.rs` (JSON schemas) → both contracts
(`contracts/mcp-tool-names.json` + `mcp-capability-map.json`, kept sorted). The
contract tests keep catalog ↔ tool-names ↔ capability-map in lockstep, so
"am I in sync?" is a test.

## Non-goals (arc sequencing)

- **Thread lifecycle + create over MCP** (`transition_thread`, `create_thread`,
  `create_channel`) — a later cluster; those publish events (FSM /
  ThreadCreated), more than a store wrapper.
- **A2 event-stream filters** (thread/member/kind on `/mcp/stream`) — Cluster
  150; pairs with these tools to complete "await my mention".

## PR ladder (actual)

| # | Title |
|---|--------|
| 149.0.1 | `feat(mcp): inbox + mention tools — agents can discover @mentions` (#388) |
| 149.0.retro | `docs(retro): Cluster 149.0 + v149.0.0 tag prep` |

## Exit criteria

- The three tools dispatch, are capability-gated, and return the right data;
  contracts in sync — **met**.
- `v149.0.0` tagged after retro.

## Verification & limits

- Unit: `inbox_tools_surface_a_members_mentions`. Contract: capability-map
  contract + capability-matrix e2e cover the new tools automatically.
  `fmt`/`clippy` clean; the generated MCP reference picks up the catalog.

## References

- [[Retros/Cluster 149.0]]; `crates/maidan-mcp/src/tools/member.rs`,
  `tools/mod.rs`, `tools/catalog.rs`.
