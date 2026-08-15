# Cluster 219.0 retro — the DAG gets a surface, and the preflight earns its checklist

> Tag **`v219.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 3.

## What shipped

- Four REST routes to build and inspect the task DAG: add / list (+`ready`) /
  remove dependency edges, and list dependents — with the full new-route preflight.

## Surprises / decisions

- **Both endpoints of an edge get RBAC.** Adding `A depends_on B` touches two
  threads, so the handler runs `ensure_thread_access` on *both* and checks they
  share a workspace — otherwise a caller who can see `A` could wire it to a `B` in
  a channel (or DM) they can't access, leaking `B`'s existence into `A`'s graph. The
  same-workspace check also keeps the DAG within a tenant.
- **`ready` rides the list response.** Rather than a separate `/ready` endpoint,
  `GET …/dependencies` returns `{ dependencies, ready }` — the two questions an
  agent asks about a task ("what am I waiting on?" and "can I start?") in one call.
- **The preflight is a real checklist, not a formality.** A new POST/DELETE route
  needs, in lockstep: the utoipa path stub, the DTO schemas in
  `components(schemas(...))`, the `http-capability-map.json` entry (or the
  `openapi_e2e` bijection fails), and — for the matrix test — both a `{dep_id}` path
  substitution *and* a POST body clause (or the extractor `400`s before `cap()` can
  `403`, failing the capability assertion). Missing any one reds a different CI job;
  doing all four up front is cheaper than three CI round-trips.
- **The matrix only proves denial.** The capability matrix hits each route with a
  *wrong-cap* bearer and asserts `403`, so the self-dependency body it sends never
  reaches the store — the body just has to deserialize. The behavioural proof lives
  in the dedicated e2e (`ready` flips, `404` on double-remove).

## Capability table extension

| Change | Where |
|--------|-------|
| DAG-management REST (`/threads/:id/dependencies` add/list+ready, `/:dep_id` remove, `/dependents`) | `routes/thread.rs`, `app.rs`, `dto.rs`, `openapi/*`, `contracts/http-capability-map.json` |

## Risks identified + still open

- **MCP surface pending** — agents can *respect* the DAG (readiness-aware
  `claim_next`, 218) but not *build* it over MCP until Cluster 220. Transitive cycle
  prevention + a "task ready" event remain later items.

## Forward look

Cluster 220 adds the MCP dependency-management tools (`add_thread_dependency`,
`list_thread_dependencies`) with the 5-place tool wiring. Then Program B's remaining
lanes: scheduled/recurring tasks, a capability registry + skill routing, queue-depth,
and coordination waits + structured results.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 218.0]].
