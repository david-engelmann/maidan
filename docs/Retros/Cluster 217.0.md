# Cluster 217.0 retro — Program B opens on the model that was already there

> Tag **`v217.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 1.

## What shipped

- The task-dependency DAG's data layer: a `maidan_thread_dependencies` edge table,
  a `ThreadDependency` model, `ThreadState::is_terminal()`, and five store methods
  (add / remove / list-dependencies / list-dependents / dependencies-satisfied) on
  both backends. No routes — a zero-blast-radius foundation.

## Surprises / decisions

- **"Task" is already "thread."** Program B is agentic orchestration, and the
  temptation is to model a new `Task` entity. But threads already are the unit of
  work — an FSM state (open/in_review/closed/archived) plus an `assignee`/claim/lease
  axis (171, 190–192). A separate task type would duplicate all of it. So the DAG is
  edges *between threads*, and "task ready" is derived from thread state. The whole
  program gets to reuse the FSM, RBAC, and claim machinery already built.
- **Readiness is a query, not stored state.** `dependencies_satisfied` computes
  readiness from the current state of the dependency threads (a single COUNT of
  non-terminal deps) rather than materializing a `ready` flag that would need
  invalidation every time a dependency transitions. Derive-don't-store keeps the
  edges as pure structure.
- **Land the foundation with no blast radius.** Cluster 159 proved the shape for a
  big feature: add the table + store with no enforcement, so it can't break anything,
  then build the surface on top. This cluster does the same — the DAG exists and is
  tested, but nothing reads it yet, so 217 is unmergeable-risk-free and 218 can focus
  entirely on the surface + the readiness-aware `claim_next`.
- **Cycle prevention: rejected the self-loop, deferred the rest.** A DB `CHECK`
  stops A→A cheaply. Transitive cycles (A→B→A) need a graph walk on every insert; I
  deferred that because a cycle's failure mode here is benign — a cyclic task never
  satisfies its dependencies, so it just never becomes ready (a deadlock the operator
  can see and break by removing an edge), not corruption.

## Capability table extension

| Change | Where |
|--------|-------|
| Task-dependency DAG store foundation (`maidan_thread_dependencies` + `thread_deps` store + `ThreadState::is_terminal`) | migrations + `store/*/thread_deps.rs` |

## Risks identified + still open

- **No surface yet** (by design) — nothing reads the DAG until Cluster 218 (REST +
  MCP + readiness-aware `claim_next`). Transitive cycle prevention + a "task ready"
  event are later items.

## Forward look

Cluster 218 adds the DAG's REST + MCP surface (add/list dependencies + dependents +
a readiness view) and makes `claim_next` **readiness-aware** — pulling only tasks
whose dependencies are all satisfied. Then Program B's remaining lanes: scheduled/
recurring tasks, a capability registry + skill routing, queue-depth, and
coordination waits + structured results.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens Program B after the
completed Program A ([[Retros/Cluster 216.0]]).
