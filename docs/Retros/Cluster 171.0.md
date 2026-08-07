# Cluster 171.0 retro — thread task assignment / handoff

> Tag **`v171.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 3 (agentic features), part 1.

## What shipped

- `Thread` gained an `assignee_id: Option<MemberId>` axis, orthogonal to the
  state FSM, with `assign` / atomic `claim` / `unassign` across both backends,
  REST (`PUT`/`DELETE /threads/:id/assignee`, `POST …/assignee/claim`), and MCP
  (`assign_thread` / `claim_thread` / `unassign_thread`).
- A `ThreadAssignmentChanged` event (prev→new assignee + actor) rides the bus, so
  subscribers/orchestrators see ownership changes live.
- All gated by the existing `thread:transition` capability + per-channel RBAC
  (`ensure_channel_access` / the MCP pre-dispatch gate).

## What was deferred / not covered

| Item | Why |
|------|-----|
| `assigned_at` / `assigned_by` columns | The event log already records actor + time. |
| "assigned-to-me" subscribe filter | New filter dimension; out of v1 scope. |
| Auto-clear assignee on archive | Assignment is deliberately orthogonal to state. |

## Surprises

- **The change was mostly SQL column-list sprawl.** `assignee_id` had to be added
  to ~16 thread SELECT/RETURNING lists across `threads.rs` + `thread_transitions.rs`
  in both backends, because they all feed the shared `row_to_thread`. The `Edit`
  `replace_all` on the exact unqualified and `t.`-qualified column strings made it
  one edit per shape per file rather than 16 hand-edits — and the compiler +
  a full test run catch any miss (a missed list yields a runtime "no column
  assignee_id" only on that path).
- **The Plan-agent pre-map paid off.** Mapping every surface (exhaustive `Event`
  matches, `federation::remap_event_workspace`, the OpenAPI bijection, both MCP
  contracts, the two capability-matrix `match`es) up front meant no iterative
  CI-failure loop like Cluster 164 — every touch-point landed in one pass.
- **The store has its OWN `parse_kind`.** `append_event` reads the just-inserted
  row back via `row_to_stored` → a `parse_kind` in `{postgres,sqlite}/events.rs`
  that is *separate* from `EventKind::parse` in `maidan-types`. Missing the new
  arm there meant the INSERT succeeded but the read-back failed, rolling back the
  tx — so events silently never persisted (`publish` swallows the append error).
  The e2e's event-count assertion caught it; the unit/contract layers can't (the
  contract test only exercises `EventKind::parse`). **Adding an event kind needs
  three parsers updated: `EventKind::parse` + both store `parse_kind`s.**

## Decisions

- **Atomic claim, not read-then-write.** The compare-and-set (`WHERE assignee_id
  IS NULL`) is the whole point — a concurrent e2e (`tokio::join!` of two claims)
  asserts exactly one winner.
- **`ThreadClaimResult { thread, claimed }` over HTTP 409.** A losing claimer
  gets `claimed:false` + the current thread, not an error to special-case.
- **Reuse `thread:transition`; single event; reject on tombstone.** All minimize
  surface while matching the workflow-control audience.

## Capability table extension

| Change | Where |
|--------|-------|
| Thread assignment/handoff/claim (REST + MCP + event), `thread:transition`-gated | `maidan-types`, `maidan-store`, `routes/thread.rs`, `maidan-mcp/src/tools/thread.rs` |

## Risks identified + still open

- **Low.** Additive column (nullable, `ON DELETE SET NULL`); reuses an existing
  capability + the established RBAC path; the only concurrency-sensitive piece
  (claim) is a single-statement CAS with a concurrent test.
- Also carried a Cluster 170 trivy-action version fix (validated on the v171
  release run) — see [[maidan-release-workflow-slow]].

## Forward look

Arc 3 continues: **structured message content** (typed blocks over
`body`/`metadata`), **MCP structured backpressure** (429 → typed retry-after),
and **HITL approvals** over the elicitation transport (which the 145–148/154
streamable work already built). Then arc 4 (token round 3).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
