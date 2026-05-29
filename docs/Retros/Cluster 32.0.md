# Cluster 32.0 retro — Helm umbrella

> Tag **`v32.0.0`**.

## What shipped

- `helm/maidan-stack` umbrella with `maidan`, optional `postgresql`, optional `minio`.
- `Chart.lock` + vendored `charts/` for offline `helm template` in CI.

## What was deferred

- `helm install` in kind/k3d CI.
- Wired `DATABASE_URL` / S3 env defaults for subchart service names.

## Forward look

Cluster 33: MCP resource notifications on HTTP tombstone and FSM transition.
