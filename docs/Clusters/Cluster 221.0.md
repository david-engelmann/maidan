# Cluster 221.0 — task-DAG transitive cycle prevention

> Program B (agentic orchestration), part 5. Phase XXIV post-gate hardening.
> Tag **`v221.0.0`**. No new gate tag.

## Goal

Harden the task-dependency DAG (Clusters 217–220) so an edge that would close a
cycle is rejected at add time. Clusters 217–220 only blocked self-loops (the
store `CHECK`); a two-node (A→B, B→A) or longer (A→B→C→A) cycle was accepted and
simply never became ready — a silent deadlock. This cluster makes the DAG
actually a DAG.

## Scope

- Store `thread_deps::add` (both backends): before inserting `thread_id →
  depends_on`, a recursive-CTE reachability check walks depends-on edges outward
  from `depends_on`; if `thread_id` is reachable, adding the edge closes a loop →
  reject with `StoreError::InvalidInput`. Check + insert share one transaction.
- Reuse the existing `InvalidInput` mapping (REST `400 BadRequest`, MCP
  `InvalidParams`) — no new error variant, route, tool, migration, or contract.
- Test: a `run_cycle_prevention_suite` (both backends) covering direct + transitive
  rejection and a valid diamond DAG acceptance, asserting the rejected edges were
  never persisted.

## Non-goals / deferred

- A "task ready" event (the reactive counterpart to readiness) — next cluster.
- Cross-workspace edges are already blocked at the REST/MCP layer (same-workspace
  check); the store guard is workspace-agnostic by design (edges are thread-keyed).

## Ordering rationale

Cycle prevention is a correctness guard on the surface just completed in 220, so it
lands immediately after — before the DAG gains reactive notifications (a "task
ready" event) or the larger Program B lanes (scheduled tasks, capability registry,
queue-depth, coordination waits).

## Risks

- **Concurrency window:** two simultaneous adds that each individually pass the
  check could together form a cycle. The shared transaction narrows this; the
  residual window is acceptable because the failure mode is a never-ready deadlock,
  not corruption (and the direct/self cases — the common ones — are fully closed).
- **Deep chains:** the recursive CTE is bounded by the DAG depth; task graphs are
  shallow in practice. No index change needed (the PK covers the join).
