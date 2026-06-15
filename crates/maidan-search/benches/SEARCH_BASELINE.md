# Search bench baseline (Cluster 109.0.2, Track U)

Run:

```sh
cargo bench -p maidan-search --bench search_hot
```

`search_hot.rs` measures **lexical (FTS5)** and **semantic (brute-force cosine)**
query latency on an in-memory SQLite store seeded with 200 messages + embeddings.
SQLite keeps it self-contained (no testcontainer) and reproducible in CI.

These numbers are **machine-specific** — treat them as a relative reference for
the Cluster 120 perf budgets, not an absolute SLA. Re-run on the target hardware
to establish the local floor.

## Reference run (Apple Silicon dev laptop, release profile, 20 samples)

| Bench | Median |
|-------|--------|
| `search/sqlite_lexical_200` | ~0.66 ms |
| `search/sqlite_semantic_200` | ~0.96 ms |

## Postgres / pgvector

Postgres lexical (`tsvector` + GIN) and semantic (`pgvector` HNSW) latency
depends on the Cluster 109.0.1 tuning knobs — `MAIDAN_HNSW_M`,
`MAIDAN_HNSW_EF_CONSTRUCTION` (build), and `MAIDAN_HNSW_EF_SEARCH` (query) — and
must be measured against a real instance with representative data volume. This
SQLite bench is the CI-friendly floor; the gate (Cluster 120) records the
Postgres budget separately.
