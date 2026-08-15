# Cluster 219.0 — task-dependency DAG: REST management surface

**Theme:** Program B (agentic orchestration), part 3 — the REST API for building and
inspecting the task DAG.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v219.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| REST: add / list (+ready) / remove dependency edges + list dependents | `routes/thread.rs`, `app.rs`, `dto.rs` |
| New-route preflight: OpenAPI paths + schemas, `http-capability-map`, matrix | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Why

Clusters 217–218 built the DAG and taught `claim_next` to respect it, but the graph
could only be built by calling the store directly. This adds the human/orchestrator
management surface so a DAG can be wired over HTTP.

## The change

Four routes on the existing thread surface:

- `POST /threads/:id/dependencies` `{ depends_on_thread_id }` → add an edge (`204`;
  `thread:transition`). Both threads must be in the **same workspace** and visible
  to the caller (`ensure_thread_access` on *both*); a cross-workspace or self
  dependency is a `400`.
- `GET /threads/:id/dependencies` → `{ dependencies, ready }` — what the task waits
  on plus the readiness flag (`workspace:read`).
- `DELETE /threads/:id/dependencies/:dep_id` → remove an edge (`204`, or `404` if
  absent; `thread:transition`).
- `GET /threads/:id/dependents` → the tasks blocked by this one (`workspace:read`).

The new-route preflight is done: utoipa path stubs + `ThreadDependenciesView` /
`AddThreadDependency` / `ThreadDependency` registered in `components(schemas(...))`,
four `http-capability-map.json` entries (bijection), and the capability-matrix
`{dep_id}` substitution + POST body clause.

## Exit criteria

- The DAG can be built, inspected (with readiness), and torn down over REST —
  **met**.
- `v219.0.0` tagged.

## Verification & limits

- `thread_dependencies_e2e`: add → self-dep `400` → list (`ready:false`) →
  dependents → close the dependency → list (`ready:true`) → remove → remove-again
  `404`.
- `openapi_e2e` (path/schema bijection) + `http_capability_matrix_e2e` (every route
  denies the wrong capability with `403`) green.
- **Limits:** MCP dependency-management tools are Cluster 220 (agents can already
  *respect* the DAG via the readiness-aware `claim_next` from 218). Transitive cycle
  prevention + a "task ready" event remain later items.

## References

- [[Retros/Cluster 219.0]]; `routes/thread.rs`. Program B: [[Roadmap]] + memory
  `maidan-next-arc-program`. Continues [[Retros/Cluster 218.0]].
