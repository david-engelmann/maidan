# Cluster 56.0 — Delivery guarantees

**Theme:** SQLite delivery cursor parity; outbox quarantine replay API.

## Problem

Cluster 13 shipped Postgres `maidan_delivery_cursor`; SQLite still no-ops cursors.
Cluster 12 quarantined poison outbox rows but recovery is manual SQL only.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | SQLite migration `0023_delivery_cursor`; real `get` / `advance` impl |
| Store | `replay_quarantined` on Postgres + SQLite outbox (workspace-scoped) |
| HTTP | `POST /workspaces/:wid/outbox/:outbox_id/replay` (`workspace:write`) |
| Tests | SQLite cursor parity; replay e2e on Postgres |
| Docs | Retro, CHANGELOG `v56.0.0`, Capabilities, Production |

## Tag

`v56.0.0`

See [[Clusters/Product Ladder 35+]] Phase VI.
