# Cluster 19.0 — Large artifacts (S3 multipart)

Cluster 18.0 closed SQLite semantic search at **`v18.0.0`**. S3 uploads today use
a single `put_object`; large blobs buffer in memory (Cluster E deferral).

> **Goal:** S3-compatible **multipart upload** with resume-friendly parts;
> HTTP API + MCP path for large artifacts.
>
> **Target tag:** `v19.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 19.0 kickoff` (this doc)                                  | —     |
| 19.0.1     | `feat(maidan-artifacts): S3 multipart upload API`                      | TBD   |
| 19.0.2     | `feat(maidan-server): multipart artifact HTTP routes`                  | TBD   |
| 19.0.3     | `test: multipart upload e2e (MinIO)`                                   | TBD   |
| 19.0.4     | `docs: large uploads in Production + Architecture`                     | TBD   |
| 19.0.retro | `docs(retro): Cluster 19.0 + v19.0.0 tag prep`                           | TBD   |

## Exit criteria

- Multipart upload completes against MinIO in CI (testcontainers).
- `POST /artifacts/multipart` lifecycle documented and tested.
- `v19.0.0` tagged after retro.

## Out of scope

- Helm chart (Cluster 24).
- MCP streamable HTTP multiplexing (Cluster 27).

## References

- [[Clusters/Product Ladder 17-27]], [[Retros/Cluster 18.0]].
