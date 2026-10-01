# Maidan Helm chart

Primary install path for the main stack. Kustomize under `k8s/` remains
for local reference.

## Quick start

```bash
helm template maidan ./helm/maidan --set contentKek="$(openssl rand -hex 32)"
helm install maidan ./helm/maidan -f ./helm/maidan/values-prod.yaml \
  --set existingSecret=maidan-secrets -n maidan --create-namespace
```

The chart refuses to render without `contentKek` (the content key-encryption
key) unless `existingSecret` names a Secret holding `MAIDAN_CONTENT_KEK`. Keep
the same KEK across upgrades: losing it makes every stored message unreadable.

`values-prod.yaml` sets `production: true`, and a production render refuses
rather than deploy something that only works on a laptop:

- the image must be a release: `image.repository` not the local
  `maidan-server`, and `image.digest` or an `image.tag` other than `dev` or
  `latest`. `values-prod.yaml` pins `ghcr.io/david-engelmann/maidan-server` at
  the newest release; `--set image.digest=sha256:…` pins it harder.
- without `existingSecret`, `secrets.DATABASE_URL` must be set and must not be
  the development default (`maidan:maidan@postgres`), and no `secrets` value may
  be empty. `existingSecret` names a Secret you create holding
  `DATABASE_URL`, `MAIDAN_CONTENT_KEK` and any other runtime secret, which keeps
  them out of values files and the release history. Setting them inline instead:
  `--set secrets.DATABASE_URL=… --set contentKek=…`.

`config` values, and `image.tag` and `image.digest`, holding the placeholder
`CHANGE_ME` fail every render, dev included. `secrets` and `contentKek` holding
it fail when `existingSecret` is unset. Each refusal names the value to set.

Production overlays (OTel, Redis quotas, S3) are documented in [PROFILES.md](PROFILES.md).

## Validation

```bash
./scripts/helm-template-smoke.sh
./scripts/helm-install-kind-smoke.sh   # requires kind, docker, helm
```

## Production TLS (cert-manager)

```bash
helm install maidan ./helm/maidan \
  -f ./helm/maidan/values-prod.yaml \
  -f ./helm/maidan/values-cert-manager.yaml \
  --set existingSecret=maidan-secrets \
  -n maidan --create-namespace
```

`values-cert-manager.yaml` layers on `values-prod.yaml`, which names the release
image; alone it refuses to render. Set
`ingress.annotations.cert-manager.io/cluster-issuer` to your ClusterIssuer name.
Requires cert-manager and an Ingress controller (e.g. nginx) in the cluster.

## Umbrella stack

```bash
helm install maidan ./helm/maidan-stack \
  -f ./helm/maidan-stack/values-prod.yaml \
  --set postgresql.auth.password="$PG_PASSWORD" \
  --set minio.auth.rootPassword="$MINIO_PASSWORD" \
  --set maidan.existingSecret=maidan-secrets \
  -n maidan --create-namespace
```

`maidan-secrets` holds `DATABASE_URL`
(`postgres://maidan:$PG_PASSWORD@maidan-postgresql:5432/maidan` for a release
named `maidan`), `S3_ACCESS_KEY_ID` (`minio`), `S3_SECRET_ACCESS_KEY`
(`$MINIO_PASSWORD`) and `MAIDAN_CONTENT_KEK`. Without it, set
`maidan.secrets.DATABASE_URL`, `maidan.secrets.S3_SECRET_ACCESS_KEY` and
`maidan.contentKek`: the stack's prod values leave them empty, and the render
refuses until they are set. `S3_ENDPOINT` names the release's MinIO Service by
itself.
