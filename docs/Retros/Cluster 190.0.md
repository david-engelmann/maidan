# Cluster 190.0 retro — an agent can find and pull its work

> Tag **`v190.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 1.

## What shipped

- `list_assigned_threads` (my-queue) + `claim_next_thread` (pull oldest
  unassigned) on both backends, over `GET /members/:id/assigned-threads` and
  `POST /channels/:cid/threads/claim-next`. Cluster 171 shipped the write side;
  this is the read side.

## Surprises / decisions

- **The atomic claim-next is backend-shaped.** Postgres wants `FOR UPDATE SKIP
  LOCKED` — the canonical concurrent work-queue trick — so parallel claimers skip
  each other's locked candidate and each gets a *distinct* thread. SQLite has no
  `SKIP LOCKED`, but it serializes writers, so a subquery-guarded `UPDATE` is
  already race-free. Same primitive, two correct implementations; forcing one SQL
  across both would have been either wrong (SQLite lacks the syntax) or slow
  (a read-then-CAS retry loop where the DB can do it in one statement).
- **A claim is an assignment, so it emits the same event.** Reusing the 171
  `ThreadAssignmentChanged` publish means a pull shows up to subscribers exactly
  like a manual assign — no new event kind, no new parser trio.

## Decisions

- **MCP tools deferred, on purpose.** The channel-scoped `claim_next` would slot
  cleanly behind the existing pre-dispatch channel-access gate, but
  `list_assigned_threads` is keyed by `member_id` — a member-scoped *aggregate*
  read the gate doesn't cover. Rather than bolt on an ad-hoc MCP RBAC filter at
  the end of the cluster, I shipped the RBAC-correct REST surface (it filters by
  `can_access_thread`) and left the MCP tools + their filtering decision to the
  next Arc-C cluster.
- **Channel-scoped claim, not workspace-wide.** A channel is the natural queue
  unit (a worker pulls from one backlog); a workspace-wide pull is a union that
  can compose on top later.

## Capability table extension

| Change | Where |
|--------|-------|
| Assignment read-side (list-mine + atomic claim-next) | `maidan-store` + `routes/thread.rs` |

## Risks identified + still open

- **Net additive.** Open (Open Work / next cluster): MCP tools + their
  member-scoped RBAC filter; no claim **lease** yet (a claimed-then-dead agent
  holds the thread forever — lease + reclaim is the next planned cluster);
  `claim_next` is channel-scoped only.

## Forward look

Arc C continues: claim leases + reclaim (dead-agent recovery), the MCP tools for
this read-side, `roots/list`, structured tool-call transcripts, `wait_for_mention`,
handoff notes, federation `parts→content`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on
[[Retros/Cluster 171.0]].
