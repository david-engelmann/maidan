# Cluster 218.0 — readiness-aware `claim_next`

**Theme:** Program B (agentic orchestration), part 2 — make the task-dependency DAG
*functional* by teaching `claim_next` to respect it.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v218.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `claim_next` + `claim_next_with_event` skip tasks with a non-terminal dependency (both backends) | `store/{postgres,sqlite}/threads.rs` |

## Why

Cluster 217 landed the DAG edges + a `dependencies_satisfied` query, but nothing
read them yet. This cluster wires readiness into the one place it matters most —
the "pull the next task" primitive. An agent calling `claim_next` should never be
handed a task that is blocked on unfinished work.

Because the existing REST `POST /channels/:cid/threads/claim-next` route and the MCP
`claim_next_thread` tool both call the same store method, this **pure store change**
makes both surfaces dependency-aware with **no new API** — the highest-value,
lowest-surface way to turn the DAG on.

## The change

The claim candidate subquery (the SQLite subquery and the Postgres
`FOR UPDATE SKIP LOCKED` CTE, in both the base `claim_next` and the Cluster-209
`claim_next_with_event`) gains a `NOT EXISTS` clause:

```
AND NOT EXISTS (
    SELECT 1 FROM maidan_thread_dependencies d
    JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
    WHERE d.thread_id = <candidate>.id AND dep.state NOT IN ('closed', 'archived')
)
```

A task with any non-terminal dependency is excluded from the candidate set, so
`claim_next` returns the oldest *ready* claimable task (or `None` if all claimable
work is blocked). Tasks with no dependencies are unaffected. The `FOR UPDATE SKIP
LOCKED` still locks only the candidate row, not its dependencies.

## Exit criteria

- `claim_next` never hands out a task blocked by an unfinished dependency, and picks
  it up once the dependency completes — **met**.
- `v218.0.0` tagged.

## Verification & limits

- `thread_deps::run_readiness_claim_suite` (both backends): an older-but-blocked task
  is skipped for a ready one; while its dependency is still open it stays unclaimed;
  closing the dependency makes it claimable next.
- Regression: `assignment_readside` (both backends) — dependency-free claiming is
  unchanged.
- **Limits:** dependency-management surface (add/list/remove edges) is still
  REST/MCP-less — that's Cluster 219. Transitive cycle prevention + a "task ready"
  event remain later items.

## References

- [[Retros/Cluster 218.0]]; `store/*/threads.rs`. Program B: [[Roadmap]] + memory
  `maidan-next-arc-program`. Continues [[Retros/Cluster 217.0]].
