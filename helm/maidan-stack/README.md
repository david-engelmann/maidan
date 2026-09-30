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
It pins the release image (`ghcr.io/david-engelmann/maidan-server`, the same tag as
`helm/maidan/values-prod.yaml`) and sets `maidan.production`, so the render refuses a
development image, an empty tag, and empty or `CHANGE_ME` credentials; the command is in the
[`maidan` chart's README](../maidan/README.md#umbrella-stack). The refusal messages name the
subchart's values (`existingSecret`, `secrets.DATABASE_URL`); in this chart they sit under
`maidan.`.

The stack renders a vendored copy of the chart, `charts/maidan-0.1.0.tgz`. After changing
`helm/maidan`, repackage it (`helm package helm/maidan -d helm/maidan-stack/charts`);
`scripts/helm-template-smoke.sh` fails while the copy differs.

CI smoke: `scripts/helm-install-kind-smoke.sh` installs `maidan` with `values-ci.yaml` on kind.
