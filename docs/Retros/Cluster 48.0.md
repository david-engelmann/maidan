# Cluster 48.0 retro — Search scale & parity

> Tag **`v48.0.0`**.

## What shipped

- `sqlite-vec` loaded per sqlx SQLite connection; `vec_distance_cosine` in semantic SQL.
- `SearchHit.score` in `[0, 1]` for cross-backend comparison within a mode.
- Production scale docs: Postgres HNSW for prod, SQLite dev parity.

## What was deferred

- `vec0` virtual tables (keep Cluster 47 per-model BLOB tables).
- Cross-mode score fusion (lexical + semantic in one list).

## Forward look

Cluster **49**: agent context export (`GET /threads/:id/context`).
