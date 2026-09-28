# Maidan Helm chart

Primary install path for the main stack. Kustomize under `k8s/` remains
for local reference.

## Quick start

```bash
helm template maidan ./helm/maidan --set contentKek="$(openssl rand -hex 32)"
helm install maidan ./helm/maidan -f ./helm/maidan/values-prod.yaml \
  --set contentKek="$KEK_FROM_YOUR_SECRET_MANAGER" -n maidan --create-namespace
```

Set `secrets.DATABASE_URL` and image coordinates before production install.
The chart refuses to render without `contentKek` (the content key-encryption
key) unless `existingSecret` names a Secret holding `MAIDAN_CONTENT_KEK`. Keep
the same KEK across upgrades: losing it makes every stored message unreadable.

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
  --set contentKek="$KEK_FROM_YOUR_SECRET_MANAGER" \
  -n maidan --create-namespace
```

Set `ingress.annotations.cert-manager.io/cluster-issuer` to your ClusterIssuer name.
Requires cert-manager and an Ingress controller (e.g. nginx) in the cluster.

## Umbrella stack

```bash
helm install maidan ./helm/maidan-stack \
  -f ./helm/maidan-stack/values-prod.yaml \
  --set maidan.contentKek="$KEK_FROM_YOUR_SECRET_MANAGER" \
  -n maidan --create-namespace
```

Substitute `RELEASE-postgresql`, `RELEASE-minio`, and passwords in `values-prod.yaml`
before install (or override with `--set`).
