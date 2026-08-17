# Cluster 233.0 — capability-registry MCP tools

> Program B (agentic orchestration), part 17. Completes **Arc E (capability
> registry + skill routing)**. Phase XXIV post-gate hardening. Tag **`v233.0.0`**.
> No new gate tag.

## Goal

Surface the capability registry to MCP-only agents — the MCP half of the REST/MCP
split (232 was REST). An agent can declare its skills and set a task's
requirements; the routing they feed already happens in `claim_next` (231). This
closes Arc E.

## Scope

| Change | Where |
|--------|-------|
| MCP `add_member_skill` / `list_member_skills` + `add_thread_required_skill` / `list_thread_required_skills` over the shared store | `tools/skill.rs` |
| 5-place wiring: dispatch, capability, channel gate (thread-skill tools), catalog, both `contracts/mcp-*.json` (sorted) | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **Same caps as the REST twin.** `add_member_skill` = `workspace:write` (registry
  setup), `add_thread_required_skill` = `thread:transition` (managing the task); both
  lists = `workspace:read`. The thread-skill tools carry `thread_id`, so they join
  the pre-dispatch channel gate; the member-skill tools take `member_id` (no channel
  to gate).
- **Explicit `member_id`, not implicit self.** `add_member_skill(member_id, skill)`
  mirrors the REST route and lets a bearer orchestrator set up any of its agents'
  skills; an agent declaring its own passes its own id. (So the test can use the
  bypass context — the tools don't stamp `auth.member_id`, unlike `create_task_schedule`.)

## Non-goals / deferred

- A "capable members for this task" discovery read (members whose skills ⊇ the task's
  requirements) — deferred (an optional orchestrator convenience; the routing is
  automatic in `claim_next`, so it isn't load-bearing). Logged in Open Work.

## Risks

- None — thin tools over the 230/231 store methods; the routing correctness lives in
  `claim_next` (231), unchanged here.
