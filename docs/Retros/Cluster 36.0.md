# Cluster 36.0 retro — `mcp-stdio` Postgres

> Tag **`v36.0.0`**.

## What shipped

- Removed SQLite-only guard on `maidan mcp-stdio`.
- Postgres path: `PgPoolOptions` → `run_postgres_migrations` → `PostgresStore` + `PostgresSearch`.
- Integration test with `pgvector/pgvector:pg17` testcontainers (skips when Docker unavailable).

## What was deferred

- Postgres bus / indexer wiring in stdio (stdio is MCP-only; no event replay).
- Semantic embedding provider selection beyond `HashV1Provider`.

## Forward look

Cluster **37**: A2A `SendStreamingMessage`.
