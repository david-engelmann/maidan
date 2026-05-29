# Cluster 48.0 — Search scale & parity

**Theme:** SQLite semantic acceleration and comparable `score` across backends.

## Problem

SQLite semantic search brute-forces cosine distance in Rust (Cluster 18 decision).
`rank` values differ by backend and mode, so clients cannot compare hits across
deployments without reading operator docs.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Search | `sqlite-vec` loaded per connection; SQL-side `vec_distance_cosine` when available |
| API | `SearchHit.score` in `[0, 1]` — comparable within a mode across Postgres and SQLite |
| Docs | Scale guidance: Postgres + HNSW for production; SQLite dev parity |

## PRs

| # | Title |
|---|-------|
| kickoff | `docs: Cluster 48.0 kickoff` (this doc) |
| 48.0.1 | `feat(maidan-search): sqlite-vec + unified score` |
| 48.0.retro | `docs(retro): Cluster 48.0 + v48.0.0 tag prep` |

## Exit criteria

- `cargo test` semantic paths pass on SQLite and Postgres.
- `SearchHit` includes `score`; OpenAPI + Architecture updated.
- `v48.0.0` tagged after retro.

## Out of scope

- `vec0` virtual tables (keep per-model BLOB tables from Cluster 47).
- Cross-mode score fusion (lexical + semantic in one ranked list).

## Tag

`v48.0.0`

See [[Clusters/Product Ladder 35+]] Phase IV.
