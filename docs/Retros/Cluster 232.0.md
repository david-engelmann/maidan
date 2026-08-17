# Cluster 232.0 retro — the registry becomes operable

> Tag **`v232.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 16 — Arc E.

## What shipped

- REST CRUD for the capability registry — declare/list/remove member skills and
  set/list/remove thread required-skills. An operator can now drive the skill
  routing (231) without touching the store.

## Surprises / decisions

- **Two entities, two caps — because they mean different things.** A *member's*
  skills are registry setup (who can do what), so `workspace:write`; a *task's*
  required skills are part of managing that task, so `thread:transition` +
  `ensure_thread_access` (the same gate as adding a dependency). Same shape (add/
  list/remove a tag), different authority — a nice reminder that "it's just CRUD"
  still has an authorization model to get right.
- **The `:skill` path param needed two preflight touches, not one.** A by-value
  `DELETE /…/skills/:skill` means the matrix test's `substitute_path` needs a
  `{skill}` replacement in *both* the `/members/` and `/threads/` branches (any
  literal works — `cap()` 403s before the skill is read), and the two POSTs need
  body clauses (else the `AddSkill` extractor 422s before `cap()`). The
  new-route-preflight memory earns its keep every time there's a POST + a novel path
  param.
- **Six routes, but uniform.** Member and thread skills are near-identical add/list/
  remove pairs, so they shipped as one cluster rather than two thin ones — the
  duplication is in the wiring (OpenAPI stubs, contract entries), which the
  bijection + matrix tests verify wholesale.

## Capability table extension

| Change | Where |
|--------|-------|
| Member-skill + thread-required-skill REST CRUD | `routes/skills.rs`, `app.rs` |

## Risks identified + still open

- None new — thin CRUD over the 230/231 store methods with standard RBAC.

## Forward look

Arc E finishes with **233**: the MCP tools (declare/list member skills, add/list
thread required-skills) + a "capable members for this task" discovery read (members
whose skills ⊇ the task's requirements). Then Arc F — coordination waits + structured
results (234–236). Then Programs C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 231.0]].
