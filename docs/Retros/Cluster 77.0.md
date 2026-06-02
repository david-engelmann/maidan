# Cluster 77.0 retro — HTTP capability map complete

> Tag **`v77.0.0`**.

## What shipped

- `contracts/http-capability-map.json` — 93 routes (HTTP + MCP/WS/A2A appendix).
- OpenAPI stubs for automation, apps, DMs, workspace context, multipart, outbox.
- `http_openapi_capability_map_contract` — bearer OpenAPI ops ↔ map (`surface: http`).
- `http_capability_matrix_e2e` — table-driven deny matrix (with documented skips for S3 multipart and admin seeding).
- [[Capability Map]] and [[Agent Integration]] HTTP CI sections.

## What was deferred

- Deny e2e for every mutating route (apps, peers, multipart) — map documents capability; tests skip where axum extractors or S3 pre-empt cap checks.
- `POST /members/{id}/inbox/read` map corrected to `workspace:read` (matches handler).

## Next

Cluster **78** — MCP streamable bidirectional mux ([[Clusters/Product Ladder 77+]]).
