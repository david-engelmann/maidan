# Cluster 233.0 retro — the registry reaches agents; Arc E closes

> Tag **`v233.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 17 — completes Arc E.

## What shipped

- MCP `add_member_skill` / `list_member_skills` + `add_thread_required_skill` /
  `list_thread_required_skills` — an agent declares its skills and sets a task's
  requirements. With this, Arc E (capability registry + skill routing) is complete
  over REST + MCP.

## Surprises / decisions

- **Four tools, all thin.** By this point in Arc E the substance (the tables, the
  claim-gate) is done; these tools are `store.{add,list}_*` wrapped in
  `content_json`, so the whole cluster is transport. That's the payoff of building
  store → claim → REST → MCP in that order: the last surface is boring.
- **Bypass worked in the test — because the tools don't stamp identity.** Unlike
  `create_task_schedule` (which stamps `created_by = auth.member_id` and so needed a
  real-member session), the skill tools take an explicit `member_id`/`thread_id`, so
  the bypass context is fine as long as the referenced rows exist. Whether a handler
  *derives* identity from auth or takes it as an arg decides whether the bypass
  harness can test it — a small but repeatable rule.
- **Deferred discovery on purpose.** The obvious "who can do this task?" read (members
  whose skills ⊇ the requirements) is a real convenience, but it isn't load-bearing:
  `claim_next` already routes automatically, so an orchestrator that wants to *pre*-
  assign can compute it from `list_thread_required_skills` + per-member skill checks.
  Shipping Arc E without it keeps the arc tight; it's logged in Open Work.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP member-skill + thread-required-skill tools | `tools/skill.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None new — the routing correctness is in `claim_next` (231); this is transport.
- **Capable-members discovery** — deferred (optional; routing is automatic).

## Forward look

**Arc E is complete** (230 foundation → 231 skill-aware claim → 232 REST → 233 MCP).
Program B's last lane is **Arc F — coordination waits + structured results** (234–236):
a task produces a structured result when done, and others read + block on it (the
sub-agent-call primitive, composing with the DAG). Then Programs C (notifications &
reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 232.0]].
