# Cluster 105.0 — Multi-replica scale-out smoke

**Theme:** Prove, in CI, that the server is correct with ≥2 replicas — and document the supported horizontal-scaling topology.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XIX · tag **`v105.0.0`**.

**Predecessor:** [[Clusters/Cluster 102.0]], [[Clusters/Cluster 103.0]], [[Clusters/Cluster 104.0]].

---

## Problem

After 102–104 the pieces for horizontal scale exist (cross-pod notifications, distributed presence, durable ephemeral state), but **nothing in CI proves they hold together** under more than one replica. Today the closest signal is the single-replica `docker compose smoke`. This cluster builds the harness that exercises the cross-replica paths and writes down the topology operators can rely on — turning "should work multi-replica" into a tested guarantee, and setting up the perf/coverage bars that the [[Clusters/Product Ladder 102+]] gate (Cluster **120**) will require.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **CI / harness** | Bring up **≥2 `maidan-server` replicas** against one Postgres + one object store behind a simple LB (or a round-robin client), reusing the `docker compose smoke` pattern. |
| **Tests** | `scale_out_smoke_e2e` covering the cross-replica paths: subscribe/notify ([[Clusters/Cluster 102.0]]), presence ([[Clusters/Cluster 103.0]]), OAuth + reindex status ([[Clusters/Cluster 104.0]]), plus post-message/read and search through the LB. |
| **CI job** | A `scale-out smoke` job — **non-required** initially; promoted to a required check at the Cluster **120** gate. |
| **Docs** | [[Production]] "Horizontal scaling" section: supported topology, what's shared (Postgres NOTIFY, store, object store), what's still pod-local (in-flight streamable sessions), and the readiness / rolling-update story (`maxUnavailable: 0`, run-on-boot migrations). |

## Non-goals

- Load / throughput benchmarking — that is the bench harness (Cluster **109**) and the gate's perf budgets (Cluster **120**).
- Autoscaling (HPA tuning) or service-mesh configuration.
- Sticky-session / session-affinity requirements — the goal is correctness *without* them.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 105.0.1 | `ci: two-replica compose harness for e2e (shared PG + object store + LB)` |
| 105.0.2 | `test(server): scale_out_smoke_e2e (notify/presence/oauth/reindex across replicas)` |
| 105.0.3 | `ci: wire scale-out smoke job (non-required)` |
| 105.0.4 | `docs(production): horizontal scaling topology` |
| 105.0.retro | `docs(retro): Cluster 105.0 + v105.0.0 tag prep` |

## Exit criteria

- CI runs the core suite against **≥2 replicas** behind an LB and it passes.
- Supported multi-replica topology is documented in [[Production]] (shared vs pod-local state, rolling-update behavior).
- The `scale-out smoke` job is wired (non-required until the Cluster **120** gate).
- `v105.0.0` tagged after retro.

## Ordering & risks

- **After 102–104** — this cluster *proves* them; it has no independent feature value.
- **Risk — CI runtime / flake:** multi-container e2e is slow and flaky-prone. Keep the suite focused on the cross-replica invariants (not a full feature re-run), reuse the existing compose-smoke harness, and gate on explicit readiness probes before asserting.
- **Risk — false confidence:** document precisely what is *not* covered (in-flight streamable session migration, perf under load) so the "scale-out" claim isn't overread.

## References

- [[Clusters/Product Ladder 102+]] Phase XIX + the `maidan-scale-1.0` gate (Cluster 120)
- [[Clusters/Cluster 102.0]], [[Clusters/Cluster 103.0]], [[Clusters/Cluster 104.0]]
- [[Production]], [[Operations]], [[Conventions]]
