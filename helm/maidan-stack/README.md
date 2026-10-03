# maidan-stack

Umbrella chart wrapping [`maidan`](../maidan) with an optional **Postgres** (pgvector) and
**MinIO**, both templates of this chart:

| Store | Enable | Image | Service |
|-------|--------|-------|---------|
| Postgres | `postgresql.enabled` | `maidan-postgres` (`docker/Dockerfile.db`, pgvector); dev values use the local `maidan-postgres:dev`, `values-prod.yaml` pins `ghcr.io/david-engelmann/maidan-postgres` to the release | `<release>-postgresql:5432` |
| MinIO | `minio.enabled` | `cgr.dev/chainguard/minio`, by digest (the one `compose.yaml` and `k8s/` run) | `<release>-minio:9000` (console `:9001`) |

Each is a single-replica StatefulSet with a ReadWriteOnce volume (`postgresql.primary.persistence`,
`minio.persistence`), non-root, with a read-only root filesystem. Postgres is probed with
`pg_isready` over TCP, MinIO with its `/minio/health/{ready,live}` endpoints. After each install and
upgrade a Job (a Helm hook) runs `mc mb --ignore-existing` for every bucket in `minio.defaultBuckets`
(comma-separated) on `cgr.dev/chainguard/minio-client`, which has no shell, so the alias is
`MC_HOST_local`.

```bash
helm template demo helm/maidan-stack --set postgresql.enabled=true --set minio.enabled=true \
  --set maidan.contentKek="$(openssl rand -hex 32)"
```

`maidan.contentKek` (or `maidan.existingSecret`) is required; see the [`maidan`](../maidan) chart.

## How the server reaches the stores

With either store enabled the chart renders `<release>-datastores`, and the server reads it after its
own ConfigMap and Secret (`maidan.extraEnvFrom`), so its keys win:

- `postgresql.enabled`: `DATABASE_URL`, built from `postgresql.auth` (percent-encoded) and naming
  `<release>-postgresql`.
- `minio.enabled`: `ARTIFACT_BACKEND=s3`, `S3_ENDPOINT` (`http://<release>-minio:9000`), `S3_BUCKET`
  (the first of `minio.defaultBuckets`), `S3_REGION=us-east-1`, and `S3_ACCESS_KEY_ID` and
  `S3_SECRET_ACCESS_KEY` (MinIO's root user and password).

You set neither in `maidan.secrets` or `maidan.config`; what those hold for the same keys is
overridden, so `kubectl get secret <release>-datastores` is what the server connects to. The
`maidan.extraEnvFrom` entry lives in this chart's `values.yaml`, and a list you set replaces it, so keep
it: the render refuses while a store is enabled and the entry is gone. With a store disabled, point
the server at your own the usual way (`maidan.secrets.DATABASE_URL`, the S3 `maidan.config` keys, or
`maidan.existingSecret`).

`postgresql.auth` is read once, when the data directory is created. Changing the password afterwards
changes what the server sends, not the database's: `ALTER ROLE` first, then upgrade. MinIO needs a root user of 3 characters or more and a
password of 8 or more, and neither may hold a colon (`MC_HOST_local` cannot carry one); the render
refuses otherwise.

A cluster-connected `helm upgrade` restarts MinIO when `minio.auth.rootUser` or
`minio.auth.rootPassword` changes. The pod annotation `checksum/rollout-nonce` is the SHA-256 of
the Secret key `rollout-nonce`, not of the password. Helm reuses the nonce already in the Secret
while the credentials match, and writes a new one when they do not. That needs a cluster
connection: `helm template` cannot see the Secret, so every offline render uses the same nonce.
Applying those manifests again does not roll MinIO, and changing the credentials in them does not
either. After an offline credential change, run `kubectl rollout restart statefulset/<release>-minio`.

`minio.persistence.enabled`, `size` and `storageClass` are fixed once the StatefulSet exists.
Kubernetes rejects an update that adds, removes or edits `volumeClaimTemplates`, so an in-place
`helm upgrade` cannot change them. When Helm is connected to the cluster the upgrade fails first
and names the live claim and the requested one. To change them, copy the buckets out (`mc mirror`
from `<release>-minio`), delete the StatefulSet without its volume
(`kubectl delete statefulset <release>-minio --cascade=orphan`), and delete PVC
`data-<release>-minio-0` when the new pod should get a new volume (turning persistence on or off,
or another storage class). Upgrade again and copy the buckets back. Growing a volume, when its
storage class allows expansion, is a patch of that PVC, not of this chart: leave `size` as it is.
This is not the Bitnami replacement in `docs/Production.md`.

## Production

`values-prod.yaml` enables both stores, pins the release images (`ghcr.io/david-engelmann/maidan-server`
and `ghcr.io/david-engelmann/maidan-postgres`, the same tag as `helm/maidan/values-prod.yaml`;
`scripts/check-deploy-pins.sh` keeps them equal), puts the server's artifacts in MinIO (no server
volume, so replicas can run on any node), and sets `maidan.production`. A production render then
refuses:

- an empty `postgresql.auth.password` or `minio.auth.rootPassword`, and the development defaults
  (`maidan`, `minioadmin`);
- the local `maidan-postgres` repository, and a `dev`, `latest` or empty tag without a digest for
  `postgresql.image`, `minio.image` or `minio.clientImage`;
- the maidan chart's own refusals: the development server image, an empty tag, empty `maidan.secrets`
  values, and, with `postgresql.enabled` off and no `maidan.existingSecret`, an unset or development
  `maidan.secrets.DATABASE_URL`.

`CHANGE_ME` in any of these values fails every render. The install command is in the
[`maidan` chart's README](../maidan/README.md#umbrella-stack). The maidan chart's refusal messages
name its own values (`existingSecret`, `secrets.DATABASE_URL`); in this chart they sit under `maidan.`.

## The vendored server chart

The stack renders a vendored copy of the chart, `charts/maidan-0.1.0.tgz`. After changing
`helm/maidan`, repackage it (`helm package helm/maidan -d helm/maidan-stack/charts`);
`scripts/helm-template-smoke.sh` fails while the copy differs. `scripts/check-deploy-contract.sh` fails
when the MinIO digests here differ from compose's and k8s's.

## CI

`scripts/helm-install-kind-smoke.sh` (the `helm install (kind)` job) installs `maidan` with
`helm/maidan/values-ci.yaml`, then this chart with `values-ci.yaml`: both stores on, the locally built
`maidan-server:dev` and `maidan-postgres:dev`. It waits for `/health/ready` to answer 200, which needs
the migrations to have run on pgvector and the artifact bucket to answer.
