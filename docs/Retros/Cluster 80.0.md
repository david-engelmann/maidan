# Cluster 80.0 retro — Delivery ops unified

> Tag **`v80.0.0`**.

## What shipped

- Unified operator routes: `GET /workspaces/:wid/deliveries`, get/replay with `?kind=webhook|automation`.
- Store list/get/replay for `maidan_webhook_deliveries` per workspace (sqlite + postgres).
- Tagged `OperatorDelivery` JSON rows; legacy `/automation/deliveries` routes unchanged.
- `delivery_ops_unified_e2e`; HTTP map + OpenAPI stubs; [[Production]] operator table.

## What was deferred

- Merging webhook and automation tables (by design).
- UI browse for DLQ (operator API only).

## Next

Cluster **81** — Subscribe grants v3 ([[Clusters/Product Ladder 77+]]).
