# Cluster 105.0 retro — Multi-replica scale-out smoke

> Tag **`v105.0.0`**. Fourth and final cluster of Phase XIX (scale-out core); closes the phase.

## What shipped

- **Boot-migration advisory lock** — `run_postgres_migrations` now holds a Postgres session advisory lock while applying, so concurrent replica starts against a fresh/upgrading database serialize instead of racing on non-transactional DDL. `concurrent_migrations` test: four pools migrate one fresh DB at once and all succeed. (105.0.1, #291)
- **Two-replica `scale` compose profile** — `maidan-r1` + `maidan-r2` on one Postgres + a dedicated unpublished `minio-scale` object store, behind an nginx round-robin LB (`docker/nginx-scale.conf`); LB host port configurable via `MAIDAN_SCALE_LB_PORT`. (105.0.1, #291)
- **`scripts/scale-out-smoke.sh`** — brings the stack up, waits for LB readiness, drives REST cross-replica paths (workspace/message round-trip; a Cluster 104 reindex job started on one replica, polled to completion on another). (105.0.1, #291)
- **CI `scale-out smoke` job** — non-required until the Cluster 120 gate; green on first run. (105.0.1, #291)
- **Docs** — `docs/Production.md` "Horizontal scaling": shared vs pod-local state, rolling-update/boot story, explicit non-coverage. (105.0.2, this PR)

## What was deferred / not covered

- **WS/SSE + OAuth cross-replica in the *container* smoke.** Resource notifications and presence need a live stream; app-OAuth needs real auth (see Surprises). All three stay proven by the in-process Rust two-replica e2es (`two_replica_resource_notification_e2e`, `two_replica_presence_e2e`, `two_replica_durable_state_e2e`). The container smoke proves the topology holds the REST paths.
- **Restart-survival assertion**, load/throughput benchmarking (Cluster 109), autoscaling, multi-region — all out of this cluster's scope.

## Surprises

- **The smoke earned its keep on the first run** by surfacing a real multi-replica boot bug: two replicas migrating a fresh DB concurrently raced on `CREATE EXTENSION` → `duplicate key value violates unique constraint "pg_extension_name_index"`, crashing a replica. `IF NOT EXISTS` does not help — its check isn't atomic against a concurrent create. Fixed with the advisory lock. This is exactly the class of bug Phase XIX exists to catch.
- **`AUTH_DISABLED` can't create apps.** The bypass middleware injects a nil-member context and ignores bearers, so `register_app`'s `created_by` FK fails (opaque 500). That blocked exercising OAuth in the container smoke under `AUTH_DISABLED`; rather than contort the harness (or weaken auth semantics), OAuth cross-replica stays covered by the real-auth Rust e2e.
- **Two local host-port collisions** (a stray MinIO on `:9001`, another project's container on `:8090`) drove two robustness improvements: the scale stack gets its own *unpublished* object store (replicas reach it over the compose network), and the LB host port is configurable. Both make the harness collision-proof and CI-clean.

## Decisions

- **Serialize boot migrations with an advisory lock, not a separate migration Job.** A session advisory lock keeps "migrations run on boot" (simple ops, no extra deploy step) while making it safe under N concurrent replicas. The distroless runtime image has no shell/curl, so an HTTP-healthcheck-gated `depends_on` ordering wasn't available anyway. See `docs/Decisions.md`.
- **Container smoke = REST topology; Rust e2es = the hard cross-replica paths.** Don't re-prove in slow containers what a fast in-process two-replica test already proves with real auth and live streams. Document the split so "scale-out smoke" isn't overread.

## Capability table extension

| Capability | Where |
|------------|-------|
| Race-free boot migrations under N replicas | `run_postgres_migrations` advisory lock, `concurrent_migrations` test |
| Tested two-replica topology (shared PG + object store + LB) | `compose.yaml` `scale` profile, `scripts/scale-out-smoke.sh`, CI `scale-out smoke` |

## Risks

- **`scale-out smoke` is non-required** until Cluster 120, so a regression won't block merges yet — watch it manually until the gate promotes it.
- **Reindex worker affinity** (a job whose owning replica dies stays `Running`) remains; flagged for the Phase XXII work-scheduling cluster.
- Multi-container CI is flake-prone; the smoke is kept to cross-replica invariants with explicit readiness gating to limit that.

## Next

**Phase XIX (scale-out core) is closed** at `v105.0.0`. Phase **XX** (Clusters 106–110 — hardening) begins; kickoff docs are drafted in [[Clusters/Product Ladder 102+]].
