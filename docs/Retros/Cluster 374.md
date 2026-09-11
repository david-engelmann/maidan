# Cluster 374 retro — P1.1c: the MCP assignment dual-write (the P0)

Cluster 374 closes the last gap in the MCP write-path parity that the
transactional-outbox migration (Clusters 205–214) was supposed to have covered.
It is **not a new feature and not a new Wave number** — it fixes a correctness
bug tracked under the existing **P1.1c** ID, surfaced by a 2026-09-10 world-class
audit of the MCP write path and verified against the live code.

## The bug (verified, not assumed)

`docs/Open Work.md` claimed *"P1.1 (MCP write-path parity) is COMPLETE — every
MCP mutation emits its domain event like REST."* That over-claimed. The **message
+ social** tools were migrated to the atomic `*_with_event` path (Clusters
333–334), but the **assignment path never was**:

- MCP `assign_thread` / `claim_thread` / `unassign_thread` / `claim_next_thread`
  / `release_claim` called the **non-`*_with_event`** store methods and then a
  **separate** `publish_event` (via a `publish_assignment` helper) — a **non-atomic
  dual-write**. A crash between the domain commit and the publish loses the event
  (the exact hazard 205–214 closed for REST).
- MCP `claim_next` called `claim_next_thread` (not `…_with_event`), so a reclaim
  of an **expired lease did not emit `ClaimExpired`**. REST's
  `claim_next_thread_with_event` emits `ClaimExpired` (for the dead holder) then
  `ThreadAssignmentChanged` in one tx (Cluster 351). On MCP, an agent reclaiming a
  dead peer's thread announced nothing about the expiry.

MCP is the **agent-primary** surface, so this was the higher-value copy of the
write-path — quietly running the pre-205 dual-write while REST was hardened.

## What shipped

- **374.1 (#757)** — point `assign` / `claim` / `unassign` / `claim_next` /
  `release_claim` at their `*_with_event` store variants + `publish_stored` (the
  atomic bus-notify — appends nothing, just hydrates + publishes the already-
  committed event). Deleted the `publish_assignment` helper. `claim_next` now
  publishes **every** returned event, so a reclaim emits `ClaimExpired` +
  `ThreadAssignmentChanged`. Bundled the cheap `StoreError::Conflict` →
  `McpError::InvalidParams` fix (was `Internal`, a `-32603` for a client error).
- **374.2** — this retro + the doc-close (mark P1.1c ✅ FIXED, banner Latest=374).

## Decisions

- **Fold into P1.1c, no new Wave number.** Per David's locked rule (unused repo ≈
  greenfield; fold audit findings into existing IDs), this is a bug fix under the
  outbox program, not a Wave-2 feature. It ships as a `vX.0.0` post-gate cluster.
- **`release_claim` came along.** Not named by the audit, but the same class —
  it had an unused `release_claim_with_event`. Migrating it too finishes the
  assignment surface in one pass.
- **The audit's out-of-scope parts were declined.** The relayed review pointed at
  pi-repo files (`/Users/david/bg/pi/*`), I-numbers, and a parallel roadmap; those
  are not Maidan Open Work and Pi is not edited from here. Only the code-level
  claims — verified against `main` — were acted on.

## Surprises

- **A sixth caller.** The first pass migrated the four named handlers + deleted
  the helper; the compiler then found `release_claim` still calling it (E0425).
  Grep for **every** caller before deleting a shared helper.
- **The existing `wait_for_claim_expired` test never exercised the real path** —
  it `publish_event`s a *synthetic* `ClaimExpired`. So nothing proved the MCP
  claim path actually emits one; the new `mcp_claim_next_reclaim_…` e2e is the
  first real coverage.

## Test evidence

- **DoD:** `mcp_claim_next_reclaim_emits_claim_expired_then_assignment` — m1
  claims with an already-past lease (`lease_secs=-1`, a dead agent), m2
  `claim_next` reclaims, and a bus subscriber sees `ClaimExpired{m1}` +
  `ThreadAssignmentChanged{m2}`. Deterministic (no sleep — `-1` forces the expiry).
- All **79** mcp lib tests (incl. the existing assignment/claim/unclaimable tests
  — no regression) + the updated Conflict→InvalidParams error test + the server
  `mcp_capability_matrix_e2e`. `clippy --all-targets -D warnings` + strict
  unwrap/expect clean.

## Forward look

**P1.1c is closed** — the MCP write path now matches REST's crash-consistency end
to end (message, social, and assignment). **Still tracked (P1.1d):** no MCP
`transition_thread` twin of the REST FSM transition — confirm whether the
omission is a deliberate land-gate (Wave 2 #22 reviewers / #25 soundcheck own the
close) or a genuine gap. **Next: Wave 2 #22** (G5 + G-dev-5 — required reviewers).

## Acknowledgements

One impl PR (#757) + this retro. Thanks to the 2026-09-10 audit for catching the
over-claim; verified against live code and folded honestly into the roadmap.
