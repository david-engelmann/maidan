# Cluster 192.0 — agentic: claim leases + reclaim (dead-agent recovery)

**Theme:** Arc C (agentic task-queue depth), part 3 — a claimed thread can now
carry a **lease**, so a claimed-then-dead agent no longer holds it forever.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v192.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `assignment_expires_at` column (pg 0035 / sqlite 0034) + `Thread` field | migrations, `maidan-types/src/models.rs`, `*/threads.rs` |
| `claim_next_thread` lease-aware (claimable = unassigned **or** expired) + `renew_claim` | `store.rs`, `{sqlite,postgres}/threads.rs` |
| REST: `lease_secs` on claim-next + `POST /threads/:id/claim/renew` | `routes/thread.rs`, `dto.rs`, `app.rs`, OpenAPI, capability map |
| MCP: `lease_secs` on `claim_next_thread` + `renew_claim` tool | `tools/thread.rs`, `mod.rs`, `catalog.rs`, contracts |

## Why

190/191 shipped pull-based claiming, but a claim was permanent: if the agent
that claimed a thread crashed, the thread stayed assigned to it forever — no
recovery. A work queue needs a **lease**: claim for a bounded time, heartbeat to
keep it, and let others reclaim it if the holder goes silent.

## The fix

- A nullable `assignment_expires_at` on the thread. `claim_next_thread` gains an
  optional `lease_secs`: sets the deadline (`None` = durable, unchanged). A thread
  is **claimable** when `assignee_id IS NULL` **or** its lease has expired — so an
  expired lease is transparently reclaimed by the next claimer, with **no separate
  reaper** needed. (Postgres keeps `FOR UPDATE SKIP LOCKED`; SQLite the
  serialized-writer guarded UPDATE.)
- `renew_claim(thread, member, lease_secs)` — the heartbeat — extends the lease,
  but only for the current assignee (`WHERE assignee_id = member`), so a member
  can't renew a lease it doesn't hold.
- Manual `assign` / claim-a-specific-thread stay **durable** (no lease) — a
  deliberate handoff isn't a work-lease.

## Exit criteria

- A claimed thread with an expired lease is reclaimable; the holder can renew, a
  non-holder can't; durable assignment unchanged — **met**.
- `v192.0.0` tagged.

## Verification & limits

- `maidan-store` `assignment_readside` (SQLite + Postgres-testcontainers): an
  already-past lease is reclaimed by the next claimer; a valid lease blocks
  further claims; the holder renews, a non-holder gets `NotFound`. Contract-sync
  (`mcp-tool-names` / `mcp-capability-map`) + `openapi_e2e` bijection +
  `http_capability_matrix` (renew route's body clause added) green.
- Limits: no server-side lease **default** (the caller sets `lease_secs`; an
  omitted lease is durable, so an agent must opt into leasing); the reclaim is
  lazy (only a subsequent `claim_next` frees an expired lease — nothing actively
  unassigns a dead holder until someone pulls). No `ThreadAssignmentChanged`
  event on a *lazy* expiry (only on the reclaim, which does publish).

## References

- [[Retros/Cluster 192.0]]; `*/threads.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program` (Arc C). Builds on [[Retros/Cluster 190.0]] +
  [[Retros/Cluster 191.0]].
