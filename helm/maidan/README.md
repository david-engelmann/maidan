# Maidan Helm chart

Primary install path for the main stack (Cluster 24). Kustomize under `k8s/` remains
for local reference.

## Quick start

```bash
helm template maidan ./helm/maidan -f ./helm/maidan/values.yaml
helm install maidan ./helm/maidan -f ./helm/maidan/values-prod.yaml -n maidan --create-namespace
```

Set `secrets.DATABASE_URL` and image coordinates before production install.

Production overlays (OTel, Redis quotas, S3) are documented in [PROFILES.md](PROFILES.md).

## Validation

```bash
./scripts/helm-template-smoke.sh
./scripts/helm-install-kind-smoke.sh   # requires kind, docker, helm
```

## Production TLS (cert-manager)

```bash
helm install maidan ./helm/maidan \
  -f ./helm/maidan/values-cert-manager.yaml \
  -n maidan --create-namespace
```

Set `ingress.annotations.cert-manager.io/cluster-issuer` to your ClusterIssuer name.
Requires cert-manager and an Ingress controller (e.g. nginx) in the cluster.

## Umbrella stack

```bash
helm install maidan ./helm/maidan-stack \
  -f ./helm/maidan-stack/values-prod.yaml \
  -n maidan --create-namespace
```

Substitute `RELEASE-postgresql`, `RELEASE-minio`, and passwords in `values-prod.yaml`
before install (or override with `--set`).
