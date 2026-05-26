# Kubernetes manifests

Kustomize-based manifests for deploying Maidan.

## Layout

```
k8s/
├── base/             # canonical Deployment, Service, StatefulSet
│   ├── kustomization.yaml
│   ├── namespace.yaml
│   ├── configmap.yaml
│   ├── secret.example.yaml      (template; never apply directly)
│   ├── postgres-statefulset.yaml
│   ├── postgres-service.yaml
│   ├── minio-statefulset.yaml
│   ├── minio-service.yaml
│   ├── maidan-server-deployment.yaml
│   ├── maidan-server-service.yaml
│   └── maidan-server-ingress.yaml
└── overlays/
    ├── dev/          # local kind/minikube cluster
    │   └── kustomization.yaml
    └── prod/         # production cluster
        └── kustomization.yaml
```

## Quickstart (dev cluster via kind)

```sh
kind create cluster --name maidan
kubectl apply -k k8s/overlays/dev
kubectl -n maidan rollout status deploy/maidan-server
kubectl -n maidan port-forward svc/maidan-server 8080:8080
curl http://localhost:8080/health
```

## Quickstart (prod cluster)

The `prod` overlay is a template. Adjust the image tags, resource
requests/limits, and the Ingress host before applying. Secrets must be
supplied separately — `base/secret.example.yaml` is a documented
placeholder, never committed with real values.

```sh
# Apply secrets out-of-band (example using sealed-secrets or external-secrets)
kubectl apply -k k8s/overlays/prod
```

## Production image digests (Track X.2)

Pin images by digest in `overlays/prod/kustomization.yaml` after each release:

```yaml
images:
  - name: maidan-server
    newName: ghcr.io/david-engelmann/maidan-server
    digest: sha256:… # from `docker buildx imagetools inspect`
```

Floating `newTag` is fine for dev; prod should not use `:latest`.

## Image references

- `maidan-server:<tag>` — produced by `crates/maidan-server/Dockerfile`.
- `maidan-postgres:<tag>` — produced by `docker/Dockerfile.db`.

Override image tags per-environment via the overlay's
`images:` block in `kustomization.yaml`.

## Secrets

Production deployments should integrate with one of:

- [sealed-secrets](https://github.com/bitnami-labs/sealed-secrets)
- [external-secrets](https://external-secrets.io/)
- AWS Secrets Manager / GCP Secret Manager / Azure Key Vault via CSI driver

The `secret.example.yaml` documents the required keys
(`DATABASE_URL`, `S3_*`, etc.) but does not contain real values.
