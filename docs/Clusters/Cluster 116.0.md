# Cluster 116.0 — Batch embedding pipeline

**Theme:** Make the embedding indexer scale: batch embed calls with bounded backpressure, keep backfill off the live path, and bound the lag metric.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXII · tag **`v116.0.0`** (opens the phase).

**Predecessor:** the per-event `EmbeddingHandler` (one `embed`/`upsert` per message) and the all-rows `reindex` backfill.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Provider** | `EmbeddingProvider::embed_batch` — default per-item fallback; OpenAI-compatible single-request override with validated ordering. |
| **Live indexer** | `BatchingEmbeddingHandler`: bounded mpsc queue + worker that flushes batches via `embed_batch` (off-runtime). Bounded channel = backpressure. |
| **Backfill** | `reindex_rows` chunks via `embed_batch`; stays on its own task, never the live queue. |
| **Metrics** | `IndexerMetrics` → `maidan_indexer_queue_depth` (bounded by capacity) + throughput gauges. |

## Non-goals

- Batch upsert (DB) — the remote embed is the expensive call; upsert batching is separable.
- Async-native remote provider — `spawn_blocking` suffices for now (Cluster 117 territory).
- A priority scheduler for backfill vs live — separate tasks + bounded live queue is enough.

## PR ladder (actual)

| # | Title |
|---|--------|
| 116.0.1 | `feat(search): batch embedding API + chunked backfill` (#319) |
| 116.0.2 | `feat(indexer): batched live embedding with bounded backpressure + lag metrics` (#319) |
| 116.0.retro | `docs(retro): Cluster 116.0 + v116.0.0 tag prep` |

## Exit criteria

- Indexer batches embed calls **with backpressure** — **met** (bounded mpsc channel).
- Large-workspace backfill on a **separate queue** so live indexing stays fresh — **met** (backfill task never enters the live queue).
- **Indexer-lag metric bounded** — **met** (`queue_depth` hard-capped by `queue_capacity`).
- `v116.0.0` tagged after retro.

## Ordering & risks

- **Provider API first (116.0.1):** `embed_batch` is the enabler for both backfill and live batching; landing it first keeps 116.0.2 focused on the queue/worker.
- **Risk — hot-path regression:** the live indexer is on every message; mitigated by the `embedding_indexer` integration test (real Postgres) asserting batched embeds land, metrics move, and the queue drains.
- **Risk — blocking provider stalls runtime:** mitigated with `spawn_blocking`.

## References

- [[Clusters/Product Ladder 102+]] Phase XXII
- [[Retros/Cluster 116.0]]
