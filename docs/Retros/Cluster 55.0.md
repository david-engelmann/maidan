# Cluster 55.0 retro — Helm production bundle

> Tag **`v55.0.0`**.

## What shipped

- Ingress template supports `ingress.annotations` (cert-manager ClusterIssuer, nginx tuning).
- `values-cert-manager.yaml` and `maidan-stack/values-prod.yaml` production bundles.
- `values-ci.yaml` + `scripts/helm-install-kind-smoke.sh` (kind + SQLite) in CI.
- Extended `helm-template-smoke.sh` for new value files.

## What was deferred

- In-cluster cert-manager install (operators bring their own issuer).
- `helm install` with Bitnami Postgres + pgvector (needs custom image or operator).
- Adding helm-kind job to branch-protection required checks (optional ops step).

## Forward look

Cluster **56**: SQLite delivery cursor parity + outbox quarantine replay API.
