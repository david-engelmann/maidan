# Cluster 47.0 retro — Per-model embedding tables

> Tag **`v47.0.0`**.

## What shipped

- Migration to `maidan_embedding_models` + per-model `maidan_emb_*` tables.
- Runtime `ensure_model` registers new models with correct pgvector dimension.
- `maidan reindex-embeddings` CLI for provider changes.

## What was deferred

- Automatic reindex on provider env change at server startup.
- SQLite `sqlite-vec` acceleration (Cluster 48).

## Forward look

Cluster **48**: search scale & parity.
