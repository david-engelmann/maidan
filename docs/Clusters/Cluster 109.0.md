# Cluster 109.0 — ANN index tuning + search bench

**Theme:** Expose the HNSW index/query parameters and add a search benchmark to anchor perf budgets.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XX · tag **`v109.0.0`**.

**Predecessor:** Per-model embeddings from [[Clusters/Cluster 86.0]] (v47 restructure); reindex from [[Clusters/Cluster 87.0]].

---

## Problem

Per-model HNSW indexes are created with **pgvector defaults** — `crates/maidan-search/src/embedding_tables.rs:107` issues `CREATE INDEX … USING hnsw (embedding vector_cosine_ops)` with no `WITH (m = …, ef_construction = …)`, and there is no runtime `ef_search` control. The recall/latency trade-off is therefore unconfigurable, and there is **no search benchmark** to measure it: `maidan-store` already ships a `criterion` bench (`crates/maidan-store/benches/store_hot.rs`), but `maidan-search` has none. Without a baseline, the Cluster **120** perf budgets would be guesswork.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Search** | Expose HNSW **build** params (`m`, `ef_construction`) at index creation and **query** param (`ef_search`) per request/connection, via config with documented defaults that match today's pgvector behavior. |
| **Bench** | A `maidan-search` `criterion` bench (mirroring `store_hot.rs`) covering lexical and semantic latency; record a committed baseline. |
| **Tests** | Index DDL includes the configured params; semantic queries set `ef_search`. |
| **Docs** | [[Query-Tuning]] HNSW section (supersede the generic "tune `ef_search`" note with concrete knobs + defaults). |

## Non-goals

- Switching ANN engines or auto-tuning.
- SQLite ANN — that is `sqlite-vec` from [[Clusters/Cluster 85.0]].
- Load/throughput testing under concurrency — that is the Cluster **120** gate budget; this cluster provides the *measurement tool*, not the SLA.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 109.0.1 | `feat(search): configurable HNSW build params + ef_search` |
| 109.0.2 | `bench(search): criterion lexical + semantic latency harness` |
| 109.0.3 | `docs(query-tuning): HNSW tuning guide` |
| 109.0.retro | `docs(retro): Cluster 109.0 + v109.0.0 tag prep` |

## Exit criteria

- HNSW `m` / `ef_construction` / `ef_search` are configurable with documented defaults; defaults preserve current recall/latency.
- A reproducible `maidan-search` bench exists and a baseline is recorded for Cluster **120**.
- `v109.0.0` tagged after retro.

## Ordering & risks

- **Before [[Clusters/Cluster 110.0]]** (fairness budgets need the bench baseline) and before the Cluster **120** perf budgets.
- **Risk — changing build params requires a rebuild:** new `m`/`ef_construction` only affect indexes built afterward. Tie param changes to the reindex job API ([[Clusters/Cluster 87.0]]); do **not** silently rebuild on boot. Document the rebuild step.
- **Risk — bench flakiness in CI:** keep the bench out of the required path (run on demand / nightly); commit baselines as reference, not a hard gate until 120.

## References

- [[Clusters/Product Ladder 102+]] Phase XX
- [[Clusters/Cluster 86.0]] (per-model tables), [[Clusters/Cluster 87.0]] (reindex), [[Clusters/Cluster 85.0]] (sqlite-vec)
- [[Query-Tuning]], [[Architecture]]
