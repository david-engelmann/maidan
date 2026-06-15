# Cluster 109.0 retro — ANN index tuning + search bench

> Tag **`v109.0.0`**. Fourth cluster of Phase XX (hot-path hardening).

## What shipped

- **Configurable HNSW knobs** — `hnsw::HnswParams { m, ef_construction, ef_search }` (env: `MAIDAN_HNSW_M` / `_EF_CONSTRUCTION` / `_EF_SEARCH`, all optional → pgvector defaults). `ensure_model_postgres` appends `WITH (m = …, ef_construction = …)` to the HNSW `CREATE INDEX`; `PostgresSearch::semantic_search` applies `ef_search` per query via a transaction-scoped `SET LOCAL`. Postgres integration test asserts the index DDL carries the params and the `ef_search` path returns hits. (109.0.1, #303)
- **Search bench** — `maidan-search/benches/search_hot.rs` (`criterion`, mirrors `store_hot.rs`): lexical (FTS5) + semantic (brute-force cosine) latency over 200 SQLite-seeded messages, with a committed `SEARCH_BASELINE.md` reference (lexical ~0.66 ms, semantic ~0.96 ms). Run on demand; off the required CI path until the Cluster 120 gate. (109.0.2, #304)
- **Docs** — `docs/Query-Tuning.md` HNSW tuning section (the three knobs, defaults, the build-vs-query distinction, and the rebuild caveat), superseding the old generic "tune ef_search" note. (109.0.3, this PR)

## What was deferred / not covered

- **Auto-tuning / ANN-engine swaps** — out of scope; concrete knobs only.
- **SQLite ANN** — that's `sqlite-vec` (Cluster 85); the bench uses SQLite's brute-force cosine as the reproducible floor.
- **Concurrency / throughput SLAs and a hard perf gate** — Cluster 120. This cluster ships the *measurement tool* + a relative baseline, not the budget.

## Surprises

- **`hash-v1` is migration-seeded.** The first integration test tuned `hash-v1`, but that model's table + HNSW index are created by migration `0020`, so `ensure_model_postgres` early-returns and never applies the configured params. The test now uses a fresh model (`tuned-v1`) so the configured index is actually built — and it's a real operational note: changing build params requires a rebuild, not just an env change, because existing indexes (including the seeded one) are untouched.
- **`SET LOCAL` needs a transaction.** `ef_search` is a GUC; on a pooled connection a bare `SET` would leak to later queries, so the query runs inside a short transaction with `SET LOCAL` (default path stays transaction-free).

## Decisions

- **Concrete env knobs with pgvector-default fallbacks**, build params gated to index creation (no silent boot-time rebuild — tie rebuilds to the reindex job). See `docs/Query-Tuning.md` and the Cluster 87 reindex API.
- **SQLite bench as the CI-reproducible floor**; Postgres/HNSW latency measured against a real instance at the gate. Bench stays off the required path (multi-container/criterion flakiness) until Cluster 120 promotes it.

## Capability table extension

| Capability | Where |
|------------|-------|
| Tunable HNSW build + query params | `hnsw::HnswParams`, `ensure_model_postgres`, `PostgresSearch::semantic_search` |
| Lexical + semantic latency bench + baseline | `maidan-search/benches/search_hot.rs`, `SEARCH_BASELINE.md` |

## Risks

- A too-low `ef_search` or `m` degrades recall silently (no error) — the bench + the Query-Tuning guidance are the guard. Defaults preserve current recall.
- Changing build params without rebuilding is a no-op on existing indexes — documented as the rebuild caveat.

## Next

Cluster **110** — per-workspace fairness budgets (query/indexer budget so one tenant can't starve others), which builds on this bench baseline. Closes Phase XX.
