# Helm production profiles (`v88.0.0`)

Composable values overlays for `helm/maidan`. Layer on `values-prod.yaml` and
`values-cert-manager.yaml` as needed.

| File | Purpose |
|------|---------|
| `values.yaml` | Dev defaults (localfs, single replica) |
| `values-prod.yaml` | Release image pin, production refusals (see README), HPA, ingress (bring your own TLS secret), persistence |
| `values-cert-manager.yaml` | Ingress TLS via cert-manager `ClusterIssuer` |
| `values-profile-s3.yaml` | External S3-compatible artifacts |
| `values-profile-redis.yaml` | `MAIDAN_RATE_LIMIT_REDIS_URL` for multi-replica quotas |
| `values-profile-otel.yaml` | JSON logs + OTLP traces/metrics (`OTLP_ENDPOINT`, `OTLP_METRICS`) |
| `values-ci.yaml` | kind smoke (SQLite, auth off) |

## Example installs

**TLS + Postgres URL + external OTel + Redis (3 replicas):**

```bash
helm upgrade --install maidan ./helm/maidan -n maidan --create-namespace \
  -f ./helm/maidan/values-prod.yaml \
  -f ./helm/maidan/values-cert-manager.yaml \
  -f ./helm/maidan/values-profile-otel.yaml \
  -f ./helm/maidan/values-profile-redis.yaml \
  --set secrets.DATABASE_URL='postgres://user:pass@host:5432/maidan' \
  --set contentKek="$KEK_FROM_YOUR_SECRET_MANAGER"
```

**Full stack (Postgres + MinIO + TLS) via umbrella chart:**

```bash
helm upgrade --install maidan ./helm/maidan-stack -n maidan --create-namespace \
  -f ./helm/maidan-stack/values-prod.yaml \
  --set postgresql.auth.password="$PG_PASSWORD" \
  --set minio.auth.rootPassword="$MINIO_PASSWORD" \
  --set maidan.existingSecret=maidan-secrets
```

`maidan-secrets` holds `DATABASE_URL`, `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` and
`MAIDAN_CONTENT_KEK`; see the [chart README](README.md#umbrella-stack).
Create the Secret with all four keys before installation. Rendering does not
verify an existing Secret or its keys.

**S3 artifacts on managed object storage:**

```bash
helm upgrade --install maidan ./helm/maidan -n maidan \
  -f ./helm/maidan/values-prod.yaml \
  -f ./helm/maidan/values-cert-manager.yaml \
  -f ./helm/maidan/values-profile-s3.yaml \
  --set secrets.DATABASE_URL='...' \
  --set secrets.S3_ACCESS_KEY_ID='...' \
  --set secrets.S3_SECRET_ACCESS_KEY='...' \
  --set contentKek="$KEK_FROM_YOUR_SECRET_MANAGER"
```

`values-profile-s3.yaml` carries `CHANGE_ME` for both S3 keys, so the render refuses until
they are set (or `existingSecret` holds them).

Set `ingress.hosts[0].host` and `ingress.tls` to your DNS name. For OpenAI-compatible
embeddings at scale, add `config.MAIDAN_EMBEDDING_PROVIDER=openai-compatible` and
provider secrets per [[Production]].
