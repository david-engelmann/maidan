# Cluster 266.0 retro — closing the arc

> Tag **`v266.0.0`**. Phase XXIV (post-gate hardening). **Program D — read-replica
> arc, part 6 (closer).** No new gate tag.

## What shipped

- `maidan_replica_lag_bytes` (the poller now samples the primary write LSN too and
  stores `current − replay`), the Production.md "Read replicas" operator/integrator
  section, and a lag assertion in the real-replica e2e. This closes the LSN
  read-replica arc — and Program D.

## Surprises / decisions

- **The poller was the natural home for lag.** It already runs every 200 ms holding
  the reader pool; giving it the primary pool too makes lag a byproduct of the same
  tick — no new task, no new query cadence. The store just exposes an atomic; the
  server translates it to a gauge (same shape as `HydrateStats` and the 265 routing
  counters).
- **Docs are the deliverable of a closer.** The code was done at 265; what a
  production operator actually needs is the *contract*: write → `Maidan-Consistency-Token`
  → echo on reads for read-your-writes, and a clear list of what routes vs what
  always hits the primary (writes, auth, control-plane). Writing that down plainly is
  the cluster's real value.
- **Don't reinvent `pg_stat_replication`.** The app gauge exists to correlate lag
  with Maidan's own routing metric on one dashboard; the docs are explicit that
  Postgres's native replication views remain the source of truth for replication
  health, so operators aren't misled into treating our gauge as authoritative.

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_replica_lag_bytes` gauge | `postgres/mod.rs`, `metrics.rs` |
| Production.md "Read replicas" section | `docs/Production.md` |

## Risks identified + still open

- Inert without a replica. The lag gauge is best-effort telemetry (poller-refreshed);
  the e2e asserts it reports a sane value against a real standby.

## Forward look

**The LSN read-replica arc (261–266) and Program D (scale & durability) are
complete.** The security-led four-program run (A security round 2, B agentic
orchestration, C notifications & reach, D scale & durability) is now fully shipped.
Remaining known work lives in [[Open Work]] / [[Remaining Work]] (e.g. optional
read-routing for search, federation egress `content→parts`, an import path for
workspace export). Await direction for the next program.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 265.0]].
