# Cluster 87.0 retro — Reindex job API

> Tag **`v87.0.0`**.

## What shipped

- `POST /operator/reindex-embeddings` (202 + in-process job) and `GET /operator/reindex-embeddings/:job_id` status polling.
- Optional `workspace_id` body scopes reindex (`workspace:write`); omit for all workspaces (`token:admin`).
- `Search::reindex_embeddings` on Postgres/SQLite; SQLite workspace filter UUID bind fix in `reindex.rs`.
- `operator_reindex_job_indexes_workspace_messages` e2e; http-capability-map entries; audit `embeddings.reindex`.

## What was deferred

- Durable job store (jobs lost on pod restart).
- Distributed reindex workers.

## Next

Cluster **88** — Helm production profiles ([[Clusters/Product Ladder 77+]]).
