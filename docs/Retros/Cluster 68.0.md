# Cluster 68.0 retro — Automation delivery guarantees

> Tag **`v68.0.0`**.

## What shipped

- `maidan_automation_deliveries` table (Postgres migration 0026, SQLite 0025) for slash-command and FSM-hook HTTP targets.
- `AutomationDeliveryWorker` with env-tuned max attempts and poll interval; metrics `maidan_automation_delivery_total` and `maidan_automation_delivery_duration_seconds`.
- Slash HTTP: sync attempt first, enqueue on failure with `retrying` + `delivery_id`. FSM HTTP: always async enqueue.
- Operator routes: list, get, replay, and `GET .../automation/dlq` for quarantined rows.
- Store + server e2e: `automation_deliveries`, `automation_delivery_e2e`, updated `fsm_hooks_e2e`.

## What was deferred

- Unifying webhooks into `maidan_automation_deliveries` (webhooks keep `maidan_webhook_deliveries`).
- Hook invocation audit table and UI for DLQ browse.
- Exactly-once delivery to external URLs (integrators remain idempotent).
- Reliable Axum `Query` bool parsing for list filters (DLQ route is the supported path).

## Forward look

Cluster **69**: Capabilities matrix complete (table-driven e2e for every MCP tool + HTTP route).
