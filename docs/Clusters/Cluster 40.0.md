# Cluster 40.0 — Mention router & inbox

**Theme:** Delivery preferences, unread cursor, `GET /members/:id/inbox`.

## Problem

Mentions exist in storage but there is no agent/human inbox surface or routing policy for who gets notified.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | Inbox cursor / unread state per member |
| Server | `GET /members/:id/inbox` |
| Router | Mention routing policies (baseline) |
| Tests | E2E inbox after mention + DM |

## Tag

`v40.0.0`

See [[Clusters/Product Ladder 35+]] Phase II.
