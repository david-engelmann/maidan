# Cluster 110.0 — Per-workspace fairness

**Theme:** Stop one workspace from starving search/indexing for every other tenant on the same instance.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XX · tag **`v110.0.0`**.

**Predecessor:** Rate limits from [[Clusters/Cluster 30.0]]; capability quotas from [[Clusters/Cluster 54.0]].

---

## Problem

Rate limiting and quotas are **per-token only** — `crates/maidan-server/src/rate_limit/mod.rs` (`RateLimitConfig`, `config_from_env`) keys on the bearer, and capability quotas ([[Clusters/Cluster 54.0]]) are per-token on MCP `tools/call`. There is **no per-workspace dimension**. On a shared Postgres, a single heavy workspace — a large semantic search loop, or a bulk embedding backfill — can monopolize the connection pool and the indexer and degrade latency for unrelated tenants. The fairness gap widens exactly as this ladder pushes more load through the system.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Limiter** | A per-workspace dimension on the existing limiter (search + write paths especially), reusing the Redis-optional / in-memory-default backend; documented per-workspace budgets. |
| **Indexer** | A per-workspace embedding-throughput budget so one workspace's backfill cannot monopolize the indexer (dovetails with the batch pipeline in [[Clusters/Cluster 116.0]]). |
| **Tests** | A noisy-neighbor test: workspace A at its cap does not increase error rate / latency for workspace B's requests. |
| **Docs** | [[Production]] fairness/limits section; [[Threat-Model]] note (resource-exhaustion / denial-of-service-by-tenant mitigation). |

## Non-goals

- Hard CPU/IO isolation between tenants — that is Postgres/infra-level (separate instances, resource groups), out of scope here.
- Billing or usage accounting.
- Cross-instance global fairness — single-instance fairness only (multi-region remains out of scope, [[Open Work]]).

## PR ladder (suggested)

| # | Title |
|---|--------|
| 110.0.1 | `feat(server): per-workspace rate/quota dimension` |
| 110.0.2 | `feat(search): per-workspace indexer throughput budget` |
| 110.0.3 | `test(server): noisy_neighbor_fairness` |
| 110.0.4 | `docs(production): tenant fairness + Threat-Model update` |
| 110.0.retro | `docs(retro): Cluster 110.0 + v110.0.0 tag prep` |

## Exit criteria

- A workspace at its configured limit **cannot** measurably degrade another workspace's search/write latency (asserted by the noisy-neighbor test).
- Per-workspace limits are documented and enforced, with generous defaults that don't regress normal use.
- `v110.0.0` tagged after retro.

## Ordering & risks

- **After [[Clusters/Cluster 109.0]]** — the bench baseline informs sane default budgets.
- **Relates to [[Clusters/Cluster 116.0]]** — the indexer budget is the policy half; the batch pipeline is the mechanism half.
- **Risk — fairness vs throughput:** start with generous defaults and make them configurable; a too-tight budget hurts legitimate large workspaces.
- **Risk — limiter on the hot path:** the per-workspace check must be cheap (in-memory token bucket / Redis) so fairness enforcement doesn't itself become a bottleneck.

## References

- [[Clusters/Product Ladder 102+]] Phase XX
- [[Clusters/Cluster 30.0]] (rate limits), [[Clusters/Cluster 54.0]] (quotas), [[Clusters/Cluster 116.0]] (batch indexer)
- [[Production]], [[Threat-Model]], [[Architecture]]
