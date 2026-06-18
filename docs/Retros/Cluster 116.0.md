# Cluster 116.0 retro — Batch embedding pipeline

> Tag **`v116.0.0`**. First cluster of **Phase XXII (Search & indexer at scale)**.

## What shipped

- **Batch embedding API** (116.0.1): `EmbeddingProvider::embed_batch(bodies)`
  with a default per-item fallback (local providers need no change) and an
  OpenAI-compatible override that issues **one** request with an `input`
  array. Response ordering is validated by a pure `parse_embeddings_batch`
  that sorts by `index` and checks count + dimension — unit-tested without a
  live endpoint.
- **Chunked backfill** (116.0.1): `reindex_rows` embeds in chunks of 32 via
  `embed_batch` instead of one request per message, with a cooperative
  `yield_now` between chunks so a large-workspace backfill stays polite.
- **Batched live indexing** (116.0.2): `BatchingEmbeddingHandler` replaces the
  per-event `EmbeddingHandler` in the production indexer. Live messages enqueue
  onto a **bounded** mpsc channel; a worker drains up to `batch_size` jobs per
  `embed_batch` call, run off-runtime via `spawn_blocking` (the remote provider
  is a blocking HTTP client).
  - **Backpressure**: the bounded channel makes `handle` await when the worker
    falls behind, slowing bus consumption rather than growing memory without
    bound.
  - **Bounded lag metric**: `queue_depth` is hard-capped by `queue_capacity`,
    so the lag gauge cannot grow unbounded.
  - **Isolation**: backfill keeps its own task and never enters the live queue,
    so a large-workspace backfill can't head-of-line-block live indexing.
- **Metrics** (116.0.2): `IndexerMetrics` (queue_depth, queue_capacity,
  embedded/failed/batches totals) shared into `AppState` and mirrored as
  `maidan_indexer_queue_depth` / `_queue_capacity` / `_embedded_total` /
  `_embed_failed_total` / `_embed_batches_total` gauges.
- **Config**: `MAIDAN_INDEXER_QUEUE_CAPACITY` (default 1024) and
  `MAIDAN_INDEXER_BATCH_SIZE` (default 32).

## What was deferred / not covered

| To           | What    | Why        |
|--------------|---------|------------|
| Cluster 117  | Pluggable production provider (`openai-compatible` first-class, tunable dim/model) | Next on the ladder. |
| (future)     | Batch the embedding **upsert** (one DB round-trip per batch) | The expensive call is the remote embed, which is now batched; upsert batching is a smaller, separable win. |
| (future)     | Move the remote provider to an async client | `spawn_blocking` already keeps the blocking client off the runtime; a native async client is a larger refactor. |
| (future)     | Batch the per-event `get_thread` liveness check | Kept per-event for parity; it's a fast indexed lookup, not the bottleneck. |

## Surprises

- **The semantic e2e tests bypass the indexer.** `search_semantic_e2e` writes
  embeddings directly via `search.upsert_embedding`, so it never exercised the
  live `EmbeddingHandler` — that path is covered only by
  `maidan-search/tests/embedding_indexer.rs`. The new batching handler needed
  its own test there (it would otherwise have shipped untested by e2e).
- **Backfill was already "separate" — the gap was batching, not queueing.**
  `reindex` already runs on its own spawned task distinct from the live
  indexer, so "separate queue so live stays fresh" was mostly satisfied; the
  real win was making both paths batch and ensuring the *live* path has the
  bounded queue (so a flood of live writes is what's bounded, and backfill
  never shares that queue).

## Decisions

- **Bounded channel as the backpressure primitive.** Rather than a custom
  rate-limiter, the mpsc channel's capacity is the backpressure: a full queue
  blocks `handle`, which slows bus consumption, which the bus already reports
  as lag. `queue_depth ≤ capacity` makes the lag metric inherently bounded.
- **`spawn_blocking` for the embed call.** The provider is `reqwest::blocking`;
  calling it from the worker via `spawn_blocking` keeps a slow remote embedder
  from stalling the async runtime. No [[Decisions]] change.
- **Keep backfill on its own task, don't build a second queue.** The live path
  owns the bounded queue; backfill batches independently. Simplest design that
  satisfies "live stays fresh" without a priority scheduler.
- **Default `embed_batch` loops `embed`.** Local/test providers (`hash-v1`)
  need zero new code; only remote providers override.

## Capability table extension

| Capability | Where |
|------------|-------|
| Batched live embedding indexer (bounded queue + backpressure) | `crates/maidan-search/src/embedding_batcher.rs` |
| Batch embedding provider API | `crates/maidan-search/src/embedding_provider.rs` (`embed_batch`) |
| Chunked backfill | `crates/maidan-search/src/reindex.rs` |
| Bounded indexer-lag + throughput metrics | `crates/maidan-server/src/metrics.rs` (`maidan_indexer_queue_depth`, …) |

## Risks identified + mitigated

- **Unbounded indexer memory under write floods.** The bounded queue caps
  in-flight work; excess load becomes bus backpressure, not OOM.
- **Backfill starving live indexing.** Backfill never enters the live queue and
  yields between chunks.

## Risks identified + still open

- **`spawn_blocking` pool saturation.** A very slow remote embedder holds a
  blocking-pool thread per in-flight batch (one at a time here, so low risk);
  Cluster 117's async-capable provider would remove this entirely.

## Forward look

Phase **XXII** continues with **Cluster 117 — pluggable production provider**:
first-class `openai-compatible` path with tunable dimension/model slotting into
the per-model table scheme (v47), with a documented migration/reindex story.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
