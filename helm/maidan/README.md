# Maidan Helm chart

Primary install path for the main stack (Cluster 24). Kustomize under `k8s/` remains
for local reference.

## Quick start

```bash
helm template maidan ./helm/maidan -f ./helm/maidan/values.yaml
helm install maidan ./helm/maidan -f ./helm/maidan/values-prod.yaml -n maidan --create-namespace
```

Set `secrets.MAIDAN_DATABASE_URL` and image coordinates before production install.

## Validation

```bash
./scripts/helm-template-smoke.sh
```
