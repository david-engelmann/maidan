# Cluster 328.0 retro — seed-from-message (MCP)

> Tag **`v328.0.0`**. Phase XXIV (post-gate hardening). **Cluster 10 of the fidelity +
> context flagship arc.** No new gate tag.

## What shipped

The MCP twin of Cluster 327: an agent can seed a new work thread from a source message
without leaving the tool surface.

- **MCP `seed_from_message`** (`tools/seed.rs`) — `{message_id, title, inclusion?,
  channel_id?}` → creates the titled child thread + the `seeded_from` reference edge (+ a
  quoting first message for `inclusion=quote`), returns the child thread. Standard 5-place
  wiring (`workspace:write`; the `message_id` pre-dispatch gate enforces source access; the
  target channel is checked in-handler; dispatch; catalog; both sorted contracts → 83 tools).

## Surprises / decisions

- **Atomic events + real-time parity.** Unlike the older MCP convention (non-event store
  write + `server.publish_event`, which appends the event *separately*), the seed tool uses
  the `*_with_event` store methods (row + event in one tx, the Cluster 205–214 outbox) and a
  local `notify` that bus-publishes the **returned** `StoredEvent` (hydrating the `Event`
  from its payload) — so MCP-originated seeds are both durably logged *and* real-time-bused,
  with no double-append. This is the MCP analogue of the REST `publish_stored`.
- **`BusEnvelope` is `maidan_types::BusEnvelope`**, not `maidan_bus::` — already in scope via
  `use maidan_types::*` (one compile round to learn).
- **First MCP thread-creating tool.** MCP had no `create_thread`; `seed_from_message` is the
  first tool that spawns a thread — appropriate, since seeding *is* the agentic gesture the
  arc is about (a bare create-thread tool isn't).

## Test evidence

`seed_from_message_tool_spawns_a_linked_child` (real-member session: child thread created,
`seeded_from` edge from child → source via `list_references_to`, quote message posted);
MCP contract-sync (83 tools) + `mcp_capability_matrix_e2e` (deny-without-cap + pass-with-cap)
green. fmt + strict clippy + `--all-targets` + bootstrap-strip clean; mdbook linkcheck green.

## Forward look

Seed-from-message is complete over REST + MCP (pointer + quote). The last flagship items:
**immutable context snapshot artifact** → flow template, plus the optional `pack`/`prefix`
inclusion (`prefix` delegates to Cluster-326 as-of replay) and a single-signal `WorkSeeded`
event.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues the fidelity + context
flagship arc ([[Open Work]]).
