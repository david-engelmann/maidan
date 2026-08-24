# Cluster 266.0 — replica-lag gauge + docs (read-replica arc closer)

> **Program D (scale & durability) — read-replica arc, part 6 (closer).** Phase XXIV
> post-gate hardening. Tag **`v266.0.0`**. No new gate tag.

## Goal

Close the LSN read-replica arc — and Program D — with the last operability pieces:
a replica-lag gauge and the operator/client documentation (config + the
`Maidan-Consistency-Token` contract + the routing policy).

## Scope

| Change | Where |
|--------|-------|
| `maidan_replica_lag_bytes` gauge (poller samples the primary write LSN too) | `postgres/mod.rs`, `metrics.rs` |
| Production.md "Read replicas" section (config, token contract, routing policy, metrics, testing) | `docs/Production.md` |
| Lag assertion added to the real-replica `read_routing` e2e | `maidan-store/tests/read_routing.rs` |

## Design decisions

- **Lag computed in the poller, from both LSNs.** The 264 poller already samples the
  replica's `pg_last_wal_replay_lsn()`; it now also reads the primary's
  `pg_current_wal_lsn()` each tick and stores `current − replay` (WAL bytes) in
  `ReadRoutingMetrics`. The metrics loop reads it into `maidan_replica_lag_bytes`.
  The store stays metrics-agnostic (a plain atomic, the `HydrateStats` pattern).
- **Docs lead with the token contract.** The one thing a client integrator must
  understand is: a write returns `Maidan-Consistency-Token`; echo it on a read for
  read-your-writes. The section also spells out exactly what routes (content GETs)
  and what never does (writes, auth reads, control-plane reads) so operators know
  the safety boundaries.
- **Complement, not replace, Postgres monitoring.** The docs point operators at
  `pg_stat_replication` alongside the app gauge — the app metric correlates lag with
  Maidan's routing behavior, but Postgres remains the source of truth for
  replication health.

## Validation

The `#[ignore]`d `read_routing` e2e (run against `scripts/replica-harness.sh`) now
also asserts the lag gauge reports a sane byte count after catch-up. Local mdbook
linkcheck clean for the new Production.md section.

## Outcome

**The LSN causality-token read-replica arc (Clusters 261–266) is complete:** LSN
primitives + a real replication harness (261), an inert reader pool (262), the
consistency token on writes (263), token-aware read routing (264), the full read
surface + a routing metric (265), and the lag gauge + docs (266) — all validated
against real streaming replication. **Program D (scale & durability) is complete.**

## Non-goals

- Routing search reads (maidan-search owns a separate pool) — optional, deferred.
