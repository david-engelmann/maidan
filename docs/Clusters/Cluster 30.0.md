# Cluster 30.0 — HTTP rate limits

**Theme:** Abuse protection on the HTTP surface (threat model T8 residual).

## Scope

| Layer | Deliverable |
|-------|-------------|
| Server | Optional global rate limit middleware (`MAIDAN_RATE_LIMIT_MAX`, `MAIDAN_RATE_LIMIT_WINDOW_SECS`) |
| Keying | Per bearer token prefix, else first `X-Forwarded-For` hop, else `anonymous` |
| Response | `429 Too Many Requests` as `application/problem+json` |
| Exempt | `/health/*`, `/metrics` |
| Docs | `Production.md`, retro, **`v30.0.0`** |

## Out of scope

- Per-capability quotas
- MCP stdio transport
- Redis/distributed limiter (single-process token bucket only)

## PR

`feat/cluster-30-rate-limits` → tag `v30.0.0` after retro.

## Tests

- `rate_limit_e2e.rs` — env-enabled limit returns 429 on burst
