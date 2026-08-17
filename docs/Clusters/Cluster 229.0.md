# Cluster 229.0 — task-schedule MCP tools

> Program B (agentic orchestration), part 13. Phase XXIV post-gate hardening.
> Tag **`v229.0.0`**. No new gate tag.

## Goal

Surface the scheduler to MCP-only agents — the MCP half of the REST/MCP split
(228 was REST). An agent can schedule its own recurring/one-shot work and see
what's scheduled, completing the scheduler subsystem (226 store → 227 worker → 228
REST → 229 MCP).

## Scope

| Change | Where |
|--------|-------|
| MCP `create_task_schedule` (`workspace:write`, channel-gated) + `list_task_schedules` (`workspace:read`, channel-filtered) over the shared store | `tools/schedule.rs` |
| 5-place wiring: dispatch, capability, channel gate (`channel_id`), catalog, both `contracts/mcp-*.json` (sorted) | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **Two tools, mirroring 220.** `create` + `list` — what an agent needs to
  self-schedule and introspect. Delete / pause-resume stay REST-only (operator
  actions; keeping the catalog lean, like 220 shipped add+list without remove).
- **`create` is channel-gated pre-dispatch** (the `channel_id` arg joins the
  channel-gate arm), and `list` filters its result by `can_access_channel` (a
  workspace-scoped aggregate the gate can't cover — same shape as
  `list_assigned_threads`), so a caller never sees or targets a private channel it
  can't access.
- **`created_by = auth.member_id`.** The schedule is owned by the calling agent —
  which is why the test uses a **real-member session** (`from_session`) rather than
  the nil-member bypass (the `created_by` FK, same as the REST cluster).

## Non-goals / deferred

- MCP delete / pause-resume (REST covers management).

## Risks

- None new — reuses the 228 store methods and the established MCP wiring; the
  scheduler's correctness lives in the store (226/227), unchanged here.
