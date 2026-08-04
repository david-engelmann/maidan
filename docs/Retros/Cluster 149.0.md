# Cluster 149.0 retro — MCP inbox + mention tools

> Tag **`v149.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> First of the MCP-agent-surface arc (**149–150**), from the next-arc research.

## What shipped

- Three MCP tools (`workspace:read`), mirroring the HTTP inbox handlers:
  `list_mentions` (`list_mentions_for_member`), `get_inbox`
  (`list_member_inbox`), `mark_inbox_read` (`advance_inbox_last_read_at` →
  returns the updated inbox). Limits clamp to (1, 500).
- Full contract wiring: `tools/member.rs` + dispatch/capability arms + catalog
  schemas + `mcp-tool-names.json` + `mcp-capability-map.json`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Later | `transition_thread` / `create_thread` / `create_channel` over MCP | They publish events (FSM / creation); more than a store wrapper — a separate cluster. |
| 150 | `/mcp/stream` thread/member/kind filters | Pairs with these tools to complete "await my mention" in real time. |

## Surprises

- **The gap was a single missing catalog entry.** The store methods, the HTTP
  routes, and even the inbound `record_mention` MCP tool all existed — but the
  read side was HTTP-only, so an MCP agent could be mentioned and never know.
  A tiny omission with an outsized effect on agent autonomy.

## Decisions

- **Mirror the HTTP capability (`workspace:read` for all three, including the
  cursor advance)** rather than invent a new gate — keeps the MCP and HTTP
  surfaces consistent, and `mark_inbox_read` is a benign cursor bump.
- **Clamp limits (1, 500)** like the context tools, rather than pass raw —
  small default (50), bounded max, a token-efficiency habit.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP `list_mentions` / `get_inbox` / `mark_inbox_read` | `crates/maidan-mcp/src/tools/member.rs`, catalog + contracts |

## Risks identified + still open

- **Low.** Additive, capability-gated tools over existing store methods; no new
  store/HTTP surface. Like all MCP tools, they take `member_id` as an arg and
  are gated by capability (not per-arg workspace scoping) — the standing MCP
  design.

## Forward look

**150** next: thread/member/kind filters on `GET /mcp/stream`, so an agent can
subscribe to "just my mentions" or "this one thread" server-side instead of
filtering the whole workspace client-side. Then the rest of the arc (thread
lifecycle over MCP; B1 lean reads; C1 live UI; D request_client).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
