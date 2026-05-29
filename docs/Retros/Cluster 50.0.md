# Cluster 50.0 retro — Outbound webhooks

> Tag **`v50.0.0`**.

## What shipped

- `POST/GET/DELETE /workspaces/:wid/webhooks` with `workspace:write` / `workspace:read`.
- `maidan_webhook_subscriptions` + `maidan_webhook_deliveries` (Postgres v21, SQLite v19).
- Background worker: bus match by `EventKind` filter → signed POST → exponential retry (16 attempts) → quarantine.
- Headers: `X-Maidan-Signature`, `X-Maidan-Event`, `X-Maidan-Delivery-Id`.
- Secrets encrypted via `FEDERATION_ENCRYPTION_KEY` (same ChaCha20-Poly1305 helper as federation peers).

## What was deferred

- Webhook delivery metrics on `/metrics` (outbox-style counters).
- MCP tool for webhook admin.
- Inbound webhook callbacks (Cluster 52 FSM hooks / slash commands).

## Forward look

Cluster **51**: slash commands.
