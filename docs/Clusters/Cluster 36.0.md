# Cluster 36.0 — `mcp-stdio` Postgres

**Theme:** Remove the SQLite-only guard on `maidan-cli mcp-stdio` so agents can attach to prod-like Postgres.

## Problem

Since Cluster H, `maidan mcp-stdio` bails when `DATABASE_URL` is not SQLite. Agent desktops and
IDE integrations need the same MCP surface against Postgres that the HTTP server uses.

## Scope

| Layer | Deliverable |
|-------|-------------|
| CLI | Wire `PostgresStore` + `PgSearch` (or equivalent) in `mcp-stdio` when URL is Postgres |
| MCP | No protocol changes — reuse `McpServer` |
| Tests | Integration test with testcontainers (skip if Docker unavailable) |
| Docs | CLI help + AGENTS.md note |

## Out of scope

- Embedded search provider configuration beyond existing server defaults
- Helm / compose changes

## Tag

`v36.0.0`

## Depends on

Cluster 35 (`v35.0.0`).

See [[Clusters/Product Ladder 35+]] Phase I.
