# Cluster 55.0 — Helm production bundle

**Theme:** Production ingress + cert-manager values; `helm install` smoke in kind CI.

## Problem

Cluster 24/32 shipped charts and template smoke only. Operators need TLS ingress
patterns and CI proof that `helm install` reaches `/health`.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Helm | `ingress.annotations`; `values-cert-manager.yaml`; `values-ci.yaml` |
| Stack | `maidan-stack/values-prod.yaml` (Postgres + MinIO + TLS ingress) |
| CI | `scripts/helm-install-kind-smoke.sh` + GitHub Actions job |
| Docs | Chart README + `docs/Production.md` cert-manager section |

## Tag

`v55.0.0`

See [[Clusters/Product Ladder 35+]] Phase VI.
