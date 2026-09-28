# maidan-stack

Umbrella chart wrapping [`maidan`](../maidan) with optional Bitnami **PostgreSQL** and **MinIO**.

```bash
helm dependency update helm/maidan-stack
helm template demo helm/maidan-stack --set postgresql.enabled=true --set minio.enabled=true \
  --set maidan.contentKek="$(openssl rand -hex 32)"
```

Set `maidan.secrets.DATABASE_URL` and S3 variables to match your release names before `helm install`.
`maidan.contentKek` (or `maidan.existingSecret`) is required; see the [`maidan`](../maidan) chart.

Production bundle with TLS ingress: `values-prod.yaml` (enable Postgres + MinIO, cert-manager annotations on `maidan.ingress`).

CI smoke: `scripts/helm-install-kind-smoke.sh` installs `maidan` with `values-ci.yaml` on kind.
