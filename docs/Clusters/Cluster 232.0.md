# Cluster 232.0 — capability-registry REST

> Program B (agentic orchestration), part 16. Arc E (capability registry + skill
> routing). Phase XXIV post-gate hardening. Tag **`v232.0.0`**. No new gate tag.

## Goal

Give Arc E its REST management surface: declare/list/remove a member's skills and
set/list/remove a task's required skills, so an operator drives the skill routing
(Cluster 231) without poking the store. MCP + a "capable members" discovery read
follow in 233.

## Scope

| Change | Where |
|--------|-------|
| `POST`/`GET /members/:id/skills`, `DELETE /members/:id/skills/:skill` | `routes/skills.rs`, `app.rs` |
| `POST`/`GET /threads/:id/required-skills`, `DELETE …/:skill` | `routes/skills.rs`, `app.rs` |
| DTO `AddSkill`; full new-route preflight (6 routes) | `dto.rs`, `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **Member skills are `workspace:write`; thread requirements are `thread:transition`.**
  Declaring an agent's skills is workspace-registry management (a bearer orchestrator
  or operator sets up its agents); setting a *task's* requirements is managing the
  task, so it matches the DAG-management cap (`thread:transition` + `ensure_thread_access`,
  like `add_thread_dependency`). Both lists are `workspace:read`.
- **`:skill` is a path segment.** Skills are tags like `code-review` (no slashes), so
  `DELETE /…/skills/:skill` is a clean by-value delete — `404` when the skill wasn't
  present (conditional), matching the store's `remove` boolean.
- **No self-only guard on member skills.** Unlike the actor-guarded mention/vote
  writes (Cluster 202), skill declaration is registry setup — a `workspace:write`
  holder manages any member's skills (the bearer-orchestrator model), so it's gated
  on the cap + workspace, not on acting-as-self.

## Non-goals / deferred

- MCP tools + a "capable members for this task" discovery read (Cluster 233).

## Risks

- None beyond the routing already shipped in 231; these routes are thin CRUD over
  the Cluster-230/231 store methods with the standard RBAC.
