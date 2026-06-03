# Cluster 85.0 retro — sqlite-vec optional

> Tag **`v85.0.0`**.

## What shipped

- `maidan-search` `default = []`; `sqlite-vec` feature enables `libsqlite3-sys` + SQL `vec_distance_cosine`.
- `maidan-server` / `maidan-cli` forward `sqlite-vec`; without it, SQLite semantic search uses brute-force cosine (unchanged behavior).
- CI job `sqlite-vec (optional feature)` proves default build omits linkage and feature build passes tests.
- `sqlite_vec` integration test uses `required-features = ["sqlite-vec"]`; [[Production]] opt-in docs.

## What was deferred

- Helm/Docker build-arg for `sqlite-vec` on custom SQLite images (compose stack uses Postgres).
- Making `sqlite-vec` default-on for `maidan-server` dev binaries.

## Next

Clusters **86–87** shipped; next **88** Helm profiles ([[Clusters/Product Ladder 77+]]).
