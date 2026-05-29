# Cluster 50.0 — Outbound webhooks

**Theme:** Signed HMAC delivery of workspace events to subscriber URLs.

## Problem

External automations (Zapier, n8n, custom agents) need push notifications
when Maidan state changes. Polling `GET /events` works but does not scale for
real-time integrations.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_webhook_subscriptions`, `maidan_webhook_deliveries` |
| HTTP | `POST/GET/DELETE /workspaces/:wid/webhooks` |
| Worker | Bus subscriber + delivery poller with retry/quarantine |
| Security | HMAC-SHA256 (`X-Maidan-Signature`), secrets encrypted at rest |

## Tag

`v50.0.0`

See [[Clusters/Product Ladder 35+]] Phase V.
