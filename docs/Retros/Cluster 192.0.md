# Cluster 192.0 retro — claims can expire now

> Tag **`v192.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 3.

## What shipped

- A lease on thread claims: `assignment_expires_at` column, `claim_next_thread`
  gains `lease_secs` and treats an expired lease as claimable, and a
  `renew_claim` heartbeat (holder-only). REST + MCP surfaces both.

## Surprises / decisions

- **"Reclaim" needs no reaper.** The instinct is a background job that scans for
  expired leases and unassigns them. But folding the check straight into
  `claim_next`'s claimability predicate (`assignee_id IS NULL OR lease expired`)
  means an expired lease is *transparently* reclaimed by whoever pulls next — no
  extra task, no schedule, no race between a reaper and a claimer. The queue
  self-heals on demand.
- **Testing expiry without sleeping.** A `lease_secs` of `-1` sets a deadline in
  the past, so the very next `claim_next` sees it as expired and reclaims it —
  deterministic, no `tokio::time::sleep`, works identically on both backends.
- **Adding a `Thread` field was cheaper than feared.** Only *one* struct literal
  existed outside the store's `row_to_thread` (a bus test) — the read model is
  almost always built from a row, not a literal. The real work was appending the
  column to ~13 `SELECT`/`RETURNING` lists across both backends (two disjoint
  `sed` patterns: qualified `t.` and unqualified).
- **The route-add matrix ripple didn't bite this time** — I added the
  `/threads/:id/claim/renew` body clause to `http_capability_matrix_e2e` up front
  (the Cluster 190 lesson, now in memory).

## Decisions

- **Lease is a `claim_next` concept only.** Manual `assign` / claim-a-specific
  stay durable — a deliberate handoff isn't a work-lease and shouldn't silently
  expire. Opt-in leasing (`lease_secs` omitted ⇒ durable) keeps 190/191 callers
  behaviour-identical.
- **Renew is holder-scoped** (`WHERE assignee_id = member`) so a member can't
  steal or refresh a lease it doesn't own.

## Capability table extension

| Change | Where |
|--------|-------|
| Claim leases (`assignment_expires_at`, lease-aware `claim_next`) + `renew_claim` (REST + MCP) | `*/threads.rs`, `routes/thread.rs`, `tools/thread.rs` |

## Risks identified + still open

- **Net additive, opt-in, backward-compatible.** Open (Open Work): no server-side
  default lease (the caller must set `lease_secs`); reclaim is lazy (only a
  subsequent `claim_next` frees an expired lease — nothing actively unassigns a
  dead holder until someone pulls); no event on a *lazy* expiry (the reclaim
  itself does publish `ThreadAssignmentChanged`).

## Forward look

Arc C continues: `roots/list` tool, structured tool-call transcripts,
`wait_for_mention`, handoff notes, federation `parts→content`. Then Arc D
(performance & scale).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on
[[Retros/Cluster 190.0]] + [[Retros/Cluster 191.0]].
