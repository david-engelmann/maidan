# Cluster 231.0 retro — the queue learns who can do what

> Tag **`v231.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 15 — Arc E.

## What shipped

- `thread_required_skills` (table + model + store CRUD) and a skill-match clause in
  `claim_next` / `claim_next_with_event`, so a task's required skills gate who can
  claim it. Skill routing works with no new claim API.

## Surprises / decisions

- **Routing rode the claim query — no new method.** The whole point of building the
  task-queue on `claim_next(channel, member)` (190) is that it already carries the
  member. Skill routing is one more `NOT EXISTS` beside the readiness one (218): "no
  required skill of this candidate is one the member lacks." So the existing REST
  `claim-next` route and `claim_next_thread` MCP tool became skill-aware for free —
  the third capability (after readiness 218 and leases 192) to compose into that one
  candidate predicate without touching a single caller.
- **The nested `NOT EXISTS` is the readable way to say ⊆.** "required skills ⊆
  declared skills" is `NOT EXISTS (required skill r WHERE NOT EXISTS (declared skill =
  r))`. It reads awkwardly but it's exactly set containment, and it makes a
  no-requirements task claimable-by-anyone for free (the outer `NOT EXISTS` is
  vacuously true over zero rows).
- **Postgres got it cheaper than SQLite.** `$1` was already the member id on
  Postgres, so the clause needed no new bind; SQLite's positional `?` chain needed one
  more `member_id` at the end — a reminder that the two dialects' parameter models
  make "add a predicate that reuses an existing value" free on one and a bind-order
  chore on the other.
- **Ran the neighbours, not just the new test.** The change is in a hot shared path
  (`claim_next`), so I ran `thread_deps` (readiness claim) and `assignment_readside`
  (plain claim) alongside `skill_routing` — all green, confirming the new clause
  composes with readiness/lease/assignment rather than shadowing them.

## Capability table extension

| Change | Where |
|--------|-------|
| `thread_required_skills` + skill-match `claim_next` clause | `migrations/*`, `store/*/thread_skills.rs`, `store/*/threads.rs` |

## Risks identified + still open

- **Claim query cost** — three correlated `NOT EXISTS` now; small graphs + indexed
  joins, no measured concern.

## Forward look

Arc E finishes with the surfaces: **232** REST (declare member skills, set thread
requirements, "capable members for this task" discovery), then **233** MCP tools.
Then Arc F — coordination waits + structured results (234–236). Then Programs C
(notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 230.0]].
