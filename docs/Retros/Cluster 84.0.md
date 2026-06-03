# Cluster 84.0 retro — Outbox relay modes

> Tag **`v84.0.0`**.

## What shipped

- `MAIDAN_OUTBOX_RELAY_MODE=notify|polled` — polled relay publishes to the process-local bus without `pg_notify`.
- `MAIDAN_OUTBOX_POLL_INTERVAL_MS`, `MAIDAN_OUTBOX_RELAY`; production startup rejects disabled relay when `MAIDAN_ENV=production`.
- SQLite `main` enables outbox + relay by default (Cluster 14 parity restored).
- NOTIFY-loss runbook in [[Production]]; `outbox_polled_relay_e2e`.

## What was deferred

- Runtime toggle of relay mode without restart.
- Cross-pod polled fan-out (by design — use notify or client replay).

## Next

Cluster **85** — optional `sqlite-vec` ([[Clusters/Product Ladder 77+]]).
