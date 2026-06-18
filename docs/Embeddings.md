# Embeddings & semantic search

How Maidan generates embeddings, how models are stored, and how to switch
embedding models in production (the migration / reindex story).

## Providers

`MAIDAN_EMBEDDING_PROVIDER` selects the active provider (default `hash-v1`):

| Value | Use | Notes |
|-------|-----|-------|
| `hash-v1` | Dev / tests / offline | SHA-256-derived 1024-d pseudo-embedding. No network, deterministic. **Not** semantically meaningful — for plumbing, not relevance. |
| `openai-compatible` | Production | Any OpenAI-style `/embeddings` endpoint (OpenAI, Azure OpenAI, vLLM, text-embeddings-inference, Ollama, …). |

### `openai-compatible` configuration

| Env | Required | Default | Meaning |
|-----|----------|---------|---------|
| `MAIDAN_EMBEDDING_ENDPOINT` | yes | — | Full URL of the embeddings endpoint. |
| `MAIDAN_EMBEDDING_MODEL` | yes | — | Model id sent as `model` (also the registry key — see below). |
| `MAIDAN_EMBEDDING_API_KEY` | no | — | Sent as `Authorization: Bearer …` when set. |
| `MAIDAN_EMBEDDING_DIM` | no | auto-detected | Output dimension. If unset, the server **probes** the endpoint once at boot to learn it. |
| `MAIDAN_EMBEDDING_TIMEOUT_SECS` | no | `15` | Per-request timeout. |

**Dimension auto-detect.** When `MAIDAN_EMBEDDING_DIM` is unset, the provider
issues one sentinel embed at startup and uses the returned vector length. This
means a wrong model id or an unreachable endpoint fails **at boot with a clear
error**, not silently on every message. Set `MAIDAN_EMBEDDING_DIM` explicitly
to skip the probe (e.g. air-gapped boot, or to assert the expected dimension) —
`text-embedding-3-small` is `1536`, `text-embedding-3-large` is `3072`, BGE/GTE
small models are `384`/`768`.

Batching knobs for the live indexer (Cluster 116) are in
[Production.md](Production.md): `MAIDAN_INDEXER_QUEUE_CAPACITY`,
`MAIDAN_INDEXER_BATCH_SIZE`.

## Per-model table scheme

Each embedding model gets its **own** table and a row in the
`maidan_embedding_models` registry (`model`, `dimension`, `table_name`). The
table is `maidan_emb_<slug>` where `<slug>` is the model id with non-alnum
characters folded to `_` (`text-embedding-3-small` → `maidan_emb_text_embedding_3_small`).

Consequences:

- **Models coexist.** Switching models does not destroy the old vectors; the
  old table stays queryable. You can run old + new side by side.
- **Dimension is pinned per model.** The registry records the dimension on
  first registration. Re-registering the same model id with a *different*
  dimension is rejected (`DimensionMismatch`) — pick a new model id instead.
- **Queries target a model.** Semantic search resolves the table for its
  `embedding_model` argument (the active provider's model is the default). A
  query against an unregistered model returns no hits rather than an error.

### Startup registration (Cluster 117)

On boot the server calls `Search::ensure_model` for the active provider, which
creates the per-model table + index and inserts the registry row if absent.
So a freshly-configured model is queryable before the first message is written,
and a `DimensionMismatch` surfaces in the startup logs. Registration is
best-effort and non-fatal: if it fails, messaging still serves and the
per-message write path retries `ensure_model` lazily.

## Switching models (migration / reindex)

1. **Choose a new model id.** Use a distinct `MAIDAN_EMBEDDING_MODEL` (a
   different real model, or a suffix like `text-embedding-3-small@v2` if you
   must re-embed under the same model with different params). Reusing an id
   with a changed dimension is rejected by design.
2. **Configure and restart.** Set the `openai-compatible` env vars and restart.
   Boot registers the new model's table (empty) and the live indexer begins
   embedding *new* messages under it immediately.
3. **Backfill existing messages** into the new model's table. Either:
   - **HTTP (operator):** `POST /operator/reindex-embeddings` with
     `{"workspace_id": "<uuid>"}` (workspace-scoped, needs `workspace.write`) or
     `{}` (whole instance, needs `token.admin`). Returns a `ReindexJob`; poll
     `GET /operator/reindex-embeddings/{job_id}` for `processed`/`failed`. The
     job re-embeds using the server's **active** provider, in batches.
   - **CLI (offline / large):** `maidan-cli reindex-embeddings --embedding-provider openai-compatible [--workspace-id <uuid>]`
     with the same `MAIDAN_EMBEDDING_*` env. Runs against its own pool, so it
     won't contend with the live server's statement-timeout cap.
4. **Verify**, then optionally **cut over reads.** Semantic search uses the
   active model by default; pass `embedding_model` to target a specific table
   during validation. Once the new model is fully backfilled, it *is* the
   default — no read-side flag needed.
5. **Clean up (optional).** The old model's table can be dropped manually once
   you're confident; nothing references it after cutover.

Backfill runs on its own task/queue and never enters the live indexer's bounded
queue (Cluster 116), so a large-workspace reindex does not delay live indexing.

### HNSW index parameters (Postgres)

The HNSW build params (`m`, `ef_construction`) are applied when a model's table
+ index are first created and are **fixed for that table** — see
[Query-Tuning.md](Query-Tuning.md) for the env vars. To change them you must
rebuild: drop the model's table and reindex, or register a new model id. Query
-time `ef_search` is tunable without a rebuild.

## See also

- [Architecture.md](Architecture.md) — where the indexer sits in the data flow.
- [Query-Tuning.md](Query-Tuning.md) — HNSW + relevance tuning.
- [Production.md](Production.md) — indexer batching + operational env.
