# Cluster 18.0 — SQLite semantic search

Cluster 17.0 closed MCP resource fan-out at **`v17.0.0`**. Postgres semantic search
shipped in minors 1.2–1.3; SQLite still returns `Unsupported` for
`Search::semantic_search`.

> **Goal:** Enable semantic search on SQLite dev deployments via `sqlite-vec`.
>
> **Target tag:** `v18.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 18.0 kickoff` (this doc)                                | —     |
| 18.0.1     | `feat(maidan-search): sqlite-vec extension + schema`                   | TBD   |
| 18.0.2     | `feat(maidan-search): SqliteSearch semantic_search impl`               | TBD   |
| 18.0.3     | `test: SQLite semantic search integration`                             | TBD   |
| 18.0.4     | `docs: semantic SQLite in Architecture + Production`                   | TBD   |
| 18.0.retro | `docs(retro): Cluster 18.0 + v18.0.0 tag prep`                         | TBD   |

## Exit criteria

- `cargo test` semantic paths pass on SQLite in-memory.
- MCP/HTTP `mode=semantic` works against SQLite-backed server in e2e.
- `v18.0.0` tagged after retro.

## Out of scope

- S3 multipart (Cluster 19).
- Per-model embedding table split.

## References

- [[Clusters/Product Ladder 17-26]], [[Retros/Cluster 17.0]].
