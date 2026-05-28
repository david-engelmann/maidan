# Cluster 19.0 retro — Large artifacts (S3 multipart)

> Closing wave for Cluster 19.0 · target tag `v19.0.0`.

Cluster 19.0 added S3-compatible multipart uploads for large artifacts on HTTP and MCP.

## What shipped

- **PR #190** — `S3Store` multipart lifecycle, REST routes under `/artifacts/multipart`,
  MCP tools `begin_artifact_multipart` / `upload_artifact_multipart_part` /
  `complete_artifact_multipart` / `abort_artifact_multipart`, MinIO test.

## What was deferred

| To         | What                              | Why                                |
|------------|-----------------------------------|------------------------------------|
| Cluster 24 | Helm chart                        | Deploy epic on ladder.             |
| Cluster 27 | MCP streamable HTTP multiplexing  | Transport finalization.            |
| Post-19.0  | Client-side 5 MiB part chunking docs in Production | Operators need runbook detail. |

## Surprises

- S3 requires every part except the last to be **≥ 5 MiB**; CI uses a single-part
  upload for small payloads.

## Forward look

Next: **Cluster 20.0** — message router (`maidan-router`). Ladder:
[[Clusters/Product Ladder 17-27]].
