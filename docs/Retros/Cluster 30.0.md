# Cluster 30.0 retro — HTTP rate limits

> Tag **`v30.0.0`** (PR #202).

## What shipped

- `MAIDAN_RATE_LIMIT_MAX` / `MAIDAN_RATE_LIMIT_WINDOW_SECS` middleware.
- `429` problem+json + `Retry-After`; `/health/*` and `/metrics` exempt.

## Forward look

Ladder 31–34 per [[Clusters/Product Ladder 30-34]].
