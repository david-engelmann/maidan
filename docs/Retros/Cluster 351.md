# Cluster 351 retro — the occupancy clocks

> Tag **`v351.0.0`**. Phase XXIV (post-gate hardening). **Wave 1 #2 of the forward
> program** ([[Open Work]]) — the standing hole (G1 + H8 + H9, EXPAND 190–192). No
> new gate tag.

A **multi-PR cluster** (351.1–351.6 + this retro), stacked-PR cascade. Gives an
orchestrator a live picture of *where* every task-thread's work sits and catches
stuck/dead agents. The centrepiece is **two clocks**: the *claim* clock (the lease,
`assignment_expires_at`) says when a claim will lapse; the *working* clock
(`work_started_at`) says when the holder actually began — together they separate a
**claimed-but-idle** agent (grabbed work, never started) from one **working**.
Fencing (a Burns resource-version token) makes the whole thing safe against a
zombie holder.

## What shipped

- **351.1 (#620) — `claim_lease_id` fencing foundation.** A nullable
  `claim_lease_id` column on `maidan_threads` (pg 0057 / sqlite 0056) + the
  `ClaimLeaseId` newtype. Zero-behaviour foundation (always `None` until 351.2).
  The full schema-column ripple — every thread `SELECT`/`RETURNING` (~40 sites incl.
  `thread_transitions.rs`), both `row_to_thread`, both import INSERTs.
- **351.2 (#622) — mint + fence the lease.** A fresh `ClaimLeaseId::new()` is minted
  on every path that sets an assignee (assign / claim / claim_next + their
  `_with_event` twins), cleared on unassign. **`renew_claim` is fenced**: the caller
  must present the matching `(assignee_id, claim_lease_id)`, so a stale holder whose
  claim was reclaimed by the next owner is rejected — the classic "the first holder
  unlocks the next owner's lock" bug is closed. A reclaim rotates the token.
- **351.3 (#623) — the working clock (`acknowledge_claim`).** A `work_started_at`
  column (pg 0058 / sqlite 0057), reset to `NULL` on every (re)claim/unassign so it
  always measures the *current* claim epoch, and stamped by `acknowledge_claim`
  (fenced by the lease; `COALESCE` = idempotent first-write). REST `POST
  /threads/:id/claim/acknowledge` + MCP `acknowledge_claim`. `renew_claim`
  deliberately does **not** touch it (extending the lease is not restarting work).
- **351.4 (#624) — the occupancy view.** `ChannelOccupancy {open, queued, claimed,
  working, blocked}` — the two-clocks refinement of `QueueDepth`, splitting held work
  into `claimed` (live lease, not acknowledged) vs `working` (acknowledged). One
  aggregate query per backend, REST `GET /channels/:cid/occupancy` + MCP
  `get_channel_occupancy`. Splitting `claimed` from `working` is the whole point —
  it surfaces a claimed-but-idle agent.
- **351.5 (#625) — `release_claim` (graceful handoff).** A fenced `unassign` that
  clears assignee + lease + working-clock in one write, so a shutting-down agent
  returns its work to the queue *immediately* instead of blocking it for the lease
  duration. `release_claim` (plain, for the MCP `publish_assignment` path) +
  `release_claim_with_event` (outbox, for REST). REST `POST
  /threads/:id/claim/release` + MCP `release_claim`.
- **351.6 (#626) — the reactive `ClaimExpired` event.** A distinct, filterable "an
  agent died" signal (full EventKind drill), emitted lazily + atomically by
  `claim_next` when it reclaims an *expired* lease: the pre-update holder is captured
  in-tx (Postgres via a `prev_assignee` column on the `next` CTE; SQLite via a
  race-free select-then-update split) and a `ClaimExpired` (naming the dead holder)
  is appended *before* the reclaim's `ThreadAssignmentChanged`. Non-federatable
  (a local clock signal). MCP `wait_for_claim_expired` long-poll (the `wait_for_ready`
  twin). A lease that expires but is never reclaimed emits nothing — the occupancy
  view still shows it.

The claim lifecycle is now **claim → acknowledge → renew → release**, every step
fenced by the lease token, with expiry surfaced reactively.

## Decisions

- **Two clocks in one cluster.** The guardrail was explicit: do not split the working
  clock into a second cluster. Both the claim clock (lease) and the working clock
  (acknowledge) land here.
- **State on `maidan_threads`, not a second table.** `work_started_at` is an
  orthogonal thread axis like `assignee_id` (`ThreadState` has no "working" state),
  so occupancy state lives on the thread — no journal table, symmetric with
  `assignment_expires_at`. The cost is the ~40-site column ripple (paid twice).
- **A distinct `ClaimExpired` event over overloading `ThreadAssignmentChanged`.** The
  reclaim's assignment event *could* have carried the expired holder in
  `previous_assignee_id` (a Cluster-209 quirk forces it to `None`), for ~zero ripple.
  We chose the distinct event so an orchestrator subscribes to exactly
  `claim_expired` with no filtering — and it stays the natural home if `release_claim`
  later wants to emit it eagerly.

## Surprises

- **The schema-column ripple bit twice.** 351.1 and 351.3 each added a thread column;
  each needed every `SELECT`/`RETURNING` crate-wide patched, including the sibling
  `thread_transitions.rs` (memory `maidan-schema-column-ripple`). A brace-matched
  Python inserter + the full `cargo test -p maidan-store` caught the misses.
- **Capturing the pre-reclaim holder differs by backend.** Postgres `RETURNING` gives
  the *post*-update row, so the expired assignee is captured with a `prev_assignee`
  column on the `next` CTE; SQLite can't, so `claim_next_with_event` became a
  select-then-update split (race-free under SQLite's serialized writers).
- **`claim_next_with_event`'s return shape changed** from `(Option<Thread>,
  Option<StoredEvent>)` to `(Option<Thread>, Vec<StoredEvent>)` — a reclaim emits two
  events, and the route publishes each in order.
- **One `doc-lazy-continuation` clippy nit** — a doc line starting with `+ ` reads as
  a markdown list item under `-D warnings`; caught in the strict lint before push.

## Test evidence

`assignment_readside` (both backends): the fence (a stale/wrong token is rejected on
renew, release, and ack); the working clock (idempotent first-write; reset on
reclaim); `run_claim_expired_suite` (a fresh claim emits only the assignment, a
reclaim emits `ClaimExpired` naming the dead holder then the assignment).
`event_log` (the outbox event paths). `run_occupancy_suite` + `channel_occupancy_e2e`
(the queued/claimed/working/blocked partition over HTTP). The MCP inline
`wait_for_claim_expired_returns_expiry_and_filters_private`. Full store+types+mcp
suites, both MCP contract-sync tests, OpenAPI bijection + capability matrix, fmt +
strict clippy + `--all-targets` + bootstrap-strip clean across all six PRs.

## Forward look

**The occupancy clocks are complete.** The remaining occupancy *thickener* — G1's
formal two-clocks + fencing invariant model (TLA+/TLC or a reduced loom/madsim
model) — was **not** built; it belongs to Program V (verification as proof), not this
row. The natural UI payoff, **N8 (session/occupancy chrome)** — surfacing
running/idle/needs-input/needs-approval/done + a capability card in the vanilla
`/ui` — is Wave 1 #4.

## Acknowledgements

Solo maintainer cluster; stacked-PR cascade + admin-merge per [[Operations]]. Closes
Wave 1 #2 of the forward program ([[Open Work]]) — the standing hole.
