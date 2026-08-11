# Cluster 190.0 — agentic: assignment read-side (my-queue + claim-next)

**Theme:** Arc C (agentic task-queue depth), part 1 — the *read* side of thread
assignment, so an agent can find and pull its work. Cluster 171 shipped only the
write side (assign / claim-specific / unassign).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v190.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `list_assigned_threads` + `claim_next_thread` (both backends) | `maidan-store/src/{sqlite,postgres}/threads.rs`, `store.rs` |
| `GET /members/:id/assigned-threads` + `POST /channels/:cid/threads/claim-next` | `routes/thread.rs`, `app.rs`, OpenAPI + `contracts/http-capability-map.json` |

## Why

Cluster 171 gave threads an `assignee_id` axis and write ops (assign / claim a
*specific* thread / unassign), but no way to **query** it: an agent couldn't list
its queue or pull the next available task. Those are the two primitives a work
queue needs.

## The fix

- **`list_assigned_threads(workspace, member)`** — the member's queue, live
  threads oldest-first, using the `idx_threads_assignee` index (Cluster 171). The
  REST route filters to threads the *caller* can access (RBAC-consistent with
  search / context).
- **`claim_next_thread(channel, member)`** — atomically assign the oldest
  unassigned live thread in a channel; `null` when there's none. Backend-optimal
  concurrency:
  - **Postgres**: `SELECT … FOR UPDATE SKIP LOCKED` in a CTE, then `UPDATE …
    RETURNING` — the canonical work-queue pattern, so parallel claimers skip each
    other's locked candidate and each gets a *distinct* thread.
  - **SQLite**: writers serialize, so a subquery-guarded `UPDATE … WHERE id =
    (SELECT oldest unassigned LIMIT 1)` can't double-assign.
  - A successful claim publishes `ThreadAssignmentChanged` (reusing the 171
    helper), so subscribers see the pull.

## Exit criteria

- An agent can list its assigned threads and atomically claim the next
  unassigned one; concurrent claimers don't collide — **met**.
- `v190.0.0` tagged.

## Verification & limits

- `maidan-store` `assignment_readside` test (SQLite + Postgres-testcontainers):
  claim-next takes the oldest, then the next, then `None`; list-mine reflects the
  claims and is member-scoped. `assignment_readside_e2e` (HTTP): claim → list →
  `null`-when-empty. `openapi_e2e` bijection green.
- Limits (tracked): **MCP tools deferred** to the next Arc-C cluster — a
  member-scoped aggregate read needs its own MCP-side RBAC filtering decision
  (the pre-dispatch channel-access gate keys on channel/thread args, not
  `member_id`). No claim *lease* yet (a claimed-then-dead agent holds the thread
  forever — lease + reclaim is the next planned cluster). `claim_next` is
  channel-scoped (no workspace-wide pull).

## References

- [[Retros/Cluster 190.0]]; `maidan-store/src/*/threads.rs`. Program: [[Roadmap]]
  + memory `maidan-next-arc-program` (Arc C). Builds on [[Retros/Cluster 171.0]].
