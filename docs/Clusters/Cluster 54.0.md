# Cluster 54.0 — Capability quotas & distributed limits

**Theme:** Per-token capability rate limits with optional Redis for multi-replica deployments.

## Problem

Cluster 30 global HTTP limits are process-local and keyed by bearer prefix, not
by capability or token identity. Operators need per-token quotas and shared
state across replicas.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_token_quotas` (token_id + capability → max/window) |
| HTTP | `quotas` on token mint; quota middleware after auth on protected routes |
| Limiter | Shared fixed-window backend: in-memory or `MAIDAN_RATE_LIMIT_REDIS_URL` |
| Global RL | Cluster 30 middleware uses the same Redis backend when configured |

## Tag

`v54.0.0`

See [[Clusters/Product Ladder 35+]] Phase VI.
