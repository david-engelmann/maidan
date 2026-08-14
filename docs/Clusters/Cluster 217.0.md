# Cluster 217.0 — task-dependency DAG (store foundation)

**Theme:** Program B (agentic orchestration), part 1 — the task-dependency DAG,
landed as a zero-blast-radius store foundation.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v217.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `maidan_thread_dependencies` table (both backends) | `migrations/{postgres/0037,sqlite/0036}_thread_dependencies.sql` + `migrate.rs` |
| `ThreadDependency` model + `ThreadState::is_terminal()` | `maidan-types/src/models.rs` |
| Store: add / remove / list-dependencies / list-dependents / dependencies-satisfied | `store/{postgres,sqlite}/thread_deps.rs`, `store.rs`, `*/mod.rs` |

## Why

Program B (agentic orchestration) builds on Maidan's **thread-as-task** model
(threads already carry an FSM state + an `assignee`/claim/lease axis from Clusters
171, 190–192). The foundation the rest of the program stands on is the **task DAG**:
which tasks block which. Scheduling, readiness-aware claiming, and coordination
waits all query it.

Following the project's pattern for large features (Cluster 159 landed the
`channel_members` model + store with **no enforcement** before the RBAC arc built on
it), this cluster lands the DAG **data layer only** — zero blast radius, no routes.
The REST/MCP surface + a readiness-aware `claim_next` follow in Cluster 218.

## The change

- **`maidan_thread_dependencies (thread_id, depends_on_thread_id, created_at)`** —
  directed edges, PK on the pair, FK-cascade on both threads, a `CHECK` rejecting
  self-loops. An index on `depends_on_thread_id` powers the reverse ("what do I
  block?") lookup and the readiness join.
- **`ThreadState::is_terminal()`** — `Closed | Archived`; a dependency counts as done
  when its thread is terminal.
- **Store methods:** `add` (idempotent; self-dep rejected), `remove` (conditional),
  `list_dependencies` (what a task waits on), `list_dependents` (what a task blocks),
  and `dependencies_satisfied` — true iff every dependency is terminal (a task with
  no dependencies is ready).

## Exit criteria

- The DAG edges + readiness query exist and round-trip on both backends — **met**.
- `v217.0.0` tagged.

## Verification & limits

- `thread_deps::run_dag_suite` (both backends): add two deps (idempotent),
  list deps/dependents, `dependencies_satisfied` flips false→false→true as each dep
  closes, self-dependency rejected, conditional remove.
- `backend_parity` / `dialect_parity` cover the new migration in lockstep.
- **Limits (v1):** transitive cycle *prevention* is deferred — a self-loop is
  rejected, but an A→B→A cycle is allowed and simply never becomes ready (deadlocks,
  doesn't corrupt). Edges are eventless (structural graph metadata); a real-time
  "task ready" event is a later item. No REST/MCP surface yet (Cluster 218).

## References

- [[Retros/Cluster 217.0]]; `store/*/thread_deps.rs`. Program B: [[Roadmap]] + memory
  `maidan-next-arc-program`. Opens Program B (agentic orchestration).
