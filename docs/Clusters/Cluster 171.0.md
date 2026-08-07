# Cluster 171.0 — agentic: thread task assignment / handoff

**Theme:** Arc 3 (agentic features), part 1 — an assignee axis on threads so
agents can claim work, hand it off, and be discovered as a task's owner.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v171.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `Thread.assignee_id: Option<MemberId>` + `ThreadClaimResult` | `maidan-types/src/models.rs` |
| `ThreadAssignmentChanged` event (+ `EventKind`, all accessors, contract, federation remap) | `maidan-types/src/events.rs`, `federation.rs`, `contracts/event-kinds.json` |
| Migration: `assignee_id` column on `maidan_threads` (both backends) | `migrations/{postgres/0033,sqlite/0032}_threads_assignee.sql` |
| Store: `assign` / atomic `claim` / `unassign` (both backends) + trait | `maidan-store/src/{postgres,sqlite}/threads.rs`, `store.rs` |
| REST: `PUT`/`DELETE /threads/:id/assignee`, `POST /threads/:id/assignee/claim` | `routes/thread.rs`, `app.rs`, `dto.rs`, OpenAPI |
| MCP: `assign_thread` / `claim_thread` / `unassign_thread` tools | `maidan-mcp/src/tools/{thread,mod,catalog}.rs` + both contracts |

## Why

Threads were a workflow object with a *state* (Open/InReview/Closed/Archived)
but no notion of *who owns the work*. For multi-agent collaboration that's the
missing primitive: an agent needs to **claim** a task (without two agents
grabbing it at once), **hand it off** to another, and let an orchestrator
**discover** who's on what. Assignment is a separate axis from the state FSM —
it persists across transitions.

## Key decisions

- **Atomic claim via compare-and-set.** `claim` is `UPDATE … SET assignee_id=?
  WHERE assignee_id IS NULL` — the row lock guarantees exactly one concurrent
  claimer wins. Returns `ThreadClaimResult { thread, claimed }` (not a 409) so a
  losing agent branches on `claimed` rather than handling an error.
- **Reuse `thread:transition`.** Assignment is the same "controls this thread's
  workflow" audience as the state transitions; no new capability (which would
  drag in the capability list + both matrix arms + docs).
- **One `ThreadAssignmentChanged` event** (prev→new) covers assign / handoff /
  claim / unassign — one taxonomy arm rather than two.
- **Reject on tombstoned.** `assign`/`claim` require `tombstoned_at IS NULL` —
  no claiming dead work.

## Non-goals

- `assigned_at` / `assigned_by` columns (the event log carries actor + time).
- An "assigned-to-me" event-filter dimension (the event's filterable member is
  the actor, matching `ThreadStateChanged`).
- Auto-clearing the assignee on archive (assignment is orthogonal to state).

## Exit criteria

- Assign/claim/unassign work over REST + MCP, gated by channel RBAC, emitting
  events; atomic claim has exactly one concurrent winner; suites green — **met**.
- `v171.0.0` tagged.

## Verification & limits

- `thread_assignment_e2e`: assign→GET reflects; claim-on-assigned → `claimed:false`
  (no steal); unassign→claim succeeds; **concurrent** claims → exactly one winner;
  3 assignment events emitted (failed claim emits none); non-member denied in a
  private channel. Capability matrices (HTTP + MCP) + OpenAPI bijection +
  event-kinds contract updated.
- Also folds a **Cluster 170 fix**: `trivy-action@v0.28.0` → `@v0.36.0` (v0.28.0
  pinned the yanked `setup-trivy@v0.2.1` and failed the v170 release run;
  v0.36.0 SHA-pins its dependency). Validated on the v171 release run.

## References

- [[Retros/Cluster 171.0]]; `maidan-types/src/events.rs`, `routes/thread.rs`,
  `maidan-mcp/src/tools/thread.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.
