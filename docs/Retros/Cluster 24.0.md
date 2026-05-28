# Cluster 24.0 retro — Deploy & scale (Helm)

> Closing wave for Cluster 24.0 · target tag `v24.0.0` (shipped with ladder PR #198).

Cluster 24.0 made Helm the primary install path for **maidan-server** on the main stack.

## What shipped

- **PR #198** (`0cffd8f`) — `helm/maidan` chart (Deployment, Service, ConfigMap, Secret,
  Ingress, PVC, HPA), `values.yaml` + `values-prod.yaml`, `scripts/helm-template-smoke.sh`
  in CI lint job, `k8s/README.md` points to Helm.

## What was deferred

| To | What | Why |
|----|------|-----|
| Post-24 | Umbrella chart (Postgres + MinIO + server) | Server-only first slice. |
| Post-24 | `helm install` smoke in kind/k3d CI | Template render only today. |
| [[Remaining Work]] | Production runbook Helm-first pass | Operators still wire DB/object store. |

## Surprises

- HPA requires metrics-server in cluster; prod values assume ingress class `nginx`.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| Helm chart for maidan-server | `v24.0.0` |

## Risks identified + still open

- Chart secrets default to example DSN — must override before prod.

## Forward look

Ladder **25–27** in #198; release tag **`v27.0.0`**. See [[Remaining Work]].

## Acknowledgements

- Maintainer merge #198.
