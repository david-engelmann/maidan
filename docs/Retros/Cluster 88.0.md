# Cluster 88.0 retro — Helm production profiles

> Tag **`v88.0.0`**.

## What shipped

- Composable Helm overlays: `values-profile-otel.yaml`, `values-profile-redis.yaml`, `values-profile-s3.yaml`.
- `helm/maidan/PROFILES.md` with layered `helm upgrade` examples.
- `scripts/helm-template-smoke.sh` extended to render profile combinations.

## What was deferred

- Hosted Helm operator / one-click Grafana bundle.
- In-chart OTel collector (operators bring their own endpoint).

## Next

Cluster **89** — OTLP metrics export ([[Clusters/Product Ladder 77+]]).
