# Cluster 218.0 retro — one SQL clause turns the DAG on

> Tag **`v218.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 2.

## What shipped

- `claim_next` (and its Cluster-209 `_with_event` variant, both backends) now skips
  any task with a non-terminal dependency — so the "pull the next task" primitive
  respects the DAG. No new API.

## Surprises / decisions

- **The whole feature turns on where the check lives.** 217 built the DAG edges +
  a readiness query but left them inert. The single highest-leverage place to
  consume them is the claim candidate query: add one `NOT EXISTS` clause and every
  work-pull — the REST `claim-next` route *and* the MCP `claim_next_thread` tool,
  which share the store method — becomes dependency-aware at once. A store change,
  zero new surface. Building the management API first (219) would have added surface
  before the DAG did anything; wiring readiness first makes the DAG immediately
  useful.
- **Filter, don't lock, the dependencies.** In the Postgres `FOR UPDATE SKIP
  LOCKED` CTE, the `NOT EXISTS` reads the dependency threads but they stay *outside*
  the main FROM, so only the candidate task row is locked — concurrent claimers
  still skip each other's candidates without contending on shared dependency rows.
- **Alias the candidate.** The correlated `NOT EXISTS` joins `maidan_threads` again
  (as `dep`), so the outer candidate needed an explicit alias (`t` in SQLite, `c` in
  the PG CTE) to keep `d.thread_id = <candidate>.id` unambiguous. A small thing that
  would silently mis-correlate if left implicit.
- **Regression is the proof.** `assignment_readside` (dependency-free claiming)
  staying green on both backends is what says the clause is additive — it only
  removes *blocked* tasks from the candidate set, nothing else.

## Capability table extension

| Change | Where |
|--------|-------|
| Readiness-aware `claim_next` (skips tasks with non-terminal dependencies) | `store/*/threads.rs` |

## Risks identified + still open

- **No management surface yet** — you can't add/list/remove dependency edges over
  REST or MCP until Cluster 219; the DAG is built via the store directly for now.
  Transitive cycle prevention + a "task ready" event remain later items.

## Forward look

Cluster 219 adds the DAG-management surface — REST (`/threads/:id/dependencies`
add/list, `/dependents`, DELETE) + MCP tools — with the full new-route preflight.
Then Program B's remaining lanes: scheduled/recurring tasks, a capability registry +
skill routing, queue-depth, and coordination waits + structured results.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 217.0]].
