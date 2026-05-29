# Cluster 54.0 retro — Capability quotas & distributed limits

> Tag **`v54.0.0`**.

## What shipped

- `maidan_token_quotas` table; `replace_token_quotas` / `list_token_quotas` on `Store`.
- Token mint accepts `quotas[]`; `AuthContext.token_id` for bearer resolution.
- Quota middleware on protected routes (after auth): route → capability → fixed window.
- `MAIDAN_RATE_LIMIT_REDIS_URL` optional Redis backend for global + per-token limits.

## What was deferred

- MCP JSON-RPC per-tool quotas.
- Session-cookie (OIDC UI) quotas — API tokens only.
- Quota admin UI and PATCH token quotas endpoint.

## Forward look

Cluster **55**: Helm production bundle.
