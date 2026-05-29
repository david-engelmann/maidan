# Cluster 47.0 — Per-model embedding tables

**Theme:** Safe mixed-dimension deployments; reindex CLI when provider changes.

## Problem

A single `maidan_message_embeddings` table fixed all vectors at 1024 dimensions, blocking mixed-model deployments.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_embedding_models` registry + per-model tables (`maidan_emb_*`) |
| Search | `ensure_model` on upsert; semantic search targets model table |
| CLI | `maidan reindex-embeddings` |

## Tag

`v47.0.0`

See [[Clusters/Product Ladder 35+]] Phase IV.
