# maidan-stack

Umbrella chart wrapping [`maidan`](../maidan) with optional Bitnami **PostgreSQL** and **MinIO**.

```bash
helm dependency update helm/maidan-stack
helm template demo helm/maidan-stack --set postgresql.enabled=true --set minio.enabled=true
```

Set `maidan.env.DATABASE_URL` and S3 variables to match your release names before `helm install`.
