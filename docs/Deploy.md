# Deploy

How to run Maidan locally and in a Kubernetes cluster. Refer to
[[Architecture]] for what each component does.

## Local: Docker Compose

### Prod-style stack

Builds the production image from `crates/maidan-server/Dockerfile` and a
custom Postgres image with pgvector + schema 0001 baked in.

```sh
docker compose up                  # postgres + minio
docker compose --profile full up   # + maidan-server
curl http://localhost:8080/health  # after maidan-server lands /health
```

### Hot-reload dev stack

```sh
docker compose -f compose.dev.yaml up
```

`maidan-server` runs under `cargo watch` with the workspace mounted as a
volume, so source edits trigger an in-container rebuild.

To leave the server outside the container and only run the deps:

```sh
docker compose -f compose.dev.yaml up postgres minio
DATABASE_URL=postgres://maidan:maidan@localhost:5432/maidan cargo run --bin maidan-server
```

### Without Docker

For pure host development against SQLite (lands in Cluster A PR #6):

```sh
DATABASE_URL=sqlite://./dev.db cargo run --bin maidan-server
```

## Kubernetes

Manifests are under `k8s/` and use [Kustomize](https://kustomize.io/).

```
k8s/
├── base/                # canonical resources
└── overlays/
    ├── dev/             # local kind/minikube
    └── prod/            # production cluster
```

### Dev cluster (kind)

```sh
kind create cluster --name maidan
# build images and load them into the cluster
docker build -t maidan-server:dev -f crates/maidan-server/Dockerfile .
docker build -t maidan-postgres:dev -f docker/Dockerfile.db .
kind load docker-image maidan-server:dev --name maidan
kind load docker-image maidan-postgres:dev --name maidan

kubectl apply -k k8s/overlays/dev
kubectl -n maidan rollout status deploy/maidan-server
kubectl -n maidan port-forward svc/maidan-server 8080:8080
curl http://localhost:8080/health
```

### Production cluster

The `prod` overlay is a template. Before applying:

1. Set the real image registry + tag in
   `k8s/overlays/prod/kustomization.yaml`.
2. Adjust the Ingress host (`maidan.example.com` placeholder) and TLS
   secret name.
3. Apply the `maidan-secrets` Secret out-of-band using
   [sealed-secrets](https://github.com/bitnami-labs/sealed-secrets),
   [external-secrets](https://external-secrets.io/), or a cloud-managed
   CSI secret provider.
4. Apply the overlay:

   ```sh
   kubectl apply -k k8s/overlays/prod
   ```

### Required secret keys

| Key                  | Required?            | Notes                                  |
|----------------------|----------------------|----------------------------------------|
| `DATABASE_URL`       | yes                  | Postgres connection string.            |
| `S3_ENDPOINT`        | only if S3 backend   | Lands in Cluster E.                    |
| `S3_BUCKET`          | only if S3 backend   |                                        |
| `S3_REGION`          | only if S3 backend   |                                        |
| `S3_ACCESS_KEY_ID`   | only if S3 backend   |                                        |
| `S3_SECRET_ACCESS_KEY` | only if S3 backend |                                        |
| `OTLP_ENDPOINT`      | optional             | OTLP exporter target; Cluster T.       |

`base/secret.example.yaml` documents the contract but contains no real
values.

## Image build matrix

| Image            | Dockerfile                                  | Purpose                |
|------------------|---------------------------------------------|------------------------|
| `maidan-server`  | `crates/maidan-server/Dockerfile`           | Production binary.     |
| `maidan-server`  | `crates/maidan-server/Dockerfile.dev`       | Dev hot-reload.        |
| `maidan-postgres`| `docker/Dockerfile.db`                      | Postgres + pgvector + schema 0001. |

## Migrations

`maidan-server` applies pending migrations on boot via
`run_postgres_migrations`. The custom `maidan-postgres` image **also**
applies schema 0001 the first time its data volume initializes — this is
a redundancy for fresh deployments. Subsequent migrations always come
from the server.
