# Cluster 221.0 retro — the DAG is finally acyclic

> Tag **`v221.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 5.

## What shipped

- `add_thread_dependency` rejects any edge that would close a cycle — direct
  (A→B, B→A) and transitive (A→B→C, C→A) — via a recursive-CTE reachability check
  before insert, in both backends. The task-dependency DAG (217–220) is now
  actually a DAG.

## Surprises / decisions

- **A silent deadlock, not a crash.** Before this, a cycle was accepted and simply
  never satisfied readiness — `claim_next` would skip the whole cycle forever with
  no error. That's the worst kind of bug: no signal. The fix turns it into an
  eager `400` at add time, where the caller can act on it.
- **Reachability, not a full cycle scan.** We don't need to detect existing cycles
  (there are none — every prior add was guarded). We only need: *would this one new
  edge create a cycle?* That's exactly "is `thread_id` already reachable from
  `depends_on`" — one recursive CTE, no table-wide traversal.
- **Reused `InvalidInput`.** It already maps to REST `400` and MCP `InvalidParams`,
  and the self-loop guard already returned it — so cycle rejection is
  indistinguishable to callers from the self-loop they already handle. No new error
  variant, no new mapping, no route/tool/contract/migration churn. The whole cluster
  is two store functions + one test suite.
- **Transaction around check + insert.** A single `INSERT` became `BEGIN → check →
  INSERT → COMMIT` so a concurrent add can't slip between the check and the write.
  The residual concurrency window (two adds that each pass alone but together cycle)
  is acceptable: the failure is a never-ready deadlock, not corruption, and the
  common direct/self cases are fully closed.

## Capability table extension

| Change | Where |
|--------|-------|
| Transitive cycle prevention in `add_thread_dependency` (recursive-CTE reachability, both backends) | `store/{sqlite,postgres}/thread_deps.rs` |

## Risks identified + still open

- **Concurrency window** (documented above) — narrow, non-corrupting, accepted.
- **No "task ready" event yet** — readiness is still a pull (`list_thread_dependencies`
  returns `ready`; `claim_next` derives it). The reactive push is the next cluster.

## Forward look

Program B continues: a "task ready" event (push readiness to a waiting agent), then
scheduled/recurring tasks, a capability registry + skill routing, queue-depth
metrics, and coordination waits + structured results. Then Programs C (notifications
& reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 220.0]].
