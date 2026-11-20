# Deploy

How to run Maidan locally and in a Kubernetes cluster. Refer to
[Architecture.md](Architecture.md) for what each component does.

## Local: Docker Compose

### Prod-style stack

Builds the production image from `crates/maidan-server/Dockerfile` and a
custom Postgres image (pgvector base; schema is applied at runtime by
`maidan-server`, not baked into the image).

```sh
docker compose up                  # postgres only
docker compose --profile full up   # + minio + maidan-server
curl http://localhost:8080/health
```

### Publishing the host port

`MAIDAN_BIND` is the address *inside* the container (`0.0.0.0:8080` by default)
and stays on 8080. What the
host sees is the published mapping, and 8080 is one of the most contended
ports on a developer machine — if something else already owns it, the bind
fails and the stack does not start at all. Override the host side:

```sh
MAIDAN_HOST_PORT=18080 docker compose --profile full up
curl http://localhost:18080/health
```

`MAIDAN_PEER_HOST_PORT` (default 8081) does the same for the second server
in the `federation` profile, and `compose.quickstart.yaml` honours
`MAIDAN_HOST_PORT` too. The scale and OTLP profiles already had their own
(`MAIDAN_SCALE_LB_PORT`, `MAIDAN_OTLP_PORT`).

**A consumer on the same Docker network needs no published port at all.**
Publishing exists to reach the server *from the host*; container-to-container
traffic goes to the service name and the container port directly — so an
adjacent compose project that joins Maidan's network reaches it at
`http://maidan-server:8080` whether or not a host mapping exists, and is
unaffected by a conflict on 8080. Publish only what a human or a
host-resident process has to reach.

### Hot-reload dev stack

```sh
docker compose -f compose.dev.yaml up
```

`maidan-server` runs under `cargo watch` with the workspace mounted as a
volume, so source edits trigger an in-container rebuild.

To leave the server outside the container and only run the deps:

```sh
docker compose -f compose.dev.yaml up postgres minio
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 \
DATABASE_URL=postgres://maidan:maidan@localhost:5432/maidan cargo run --bin maidan-server
```

### Without Docker (SQLite)

For pure host development against SQLite — no `docker compose` needed:

```sh
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 \
DATABASE_URL='sqlite://./dev.db?mode=rwc' cargo run --bin maidan-server
```

Or against an in-memory SQLite (lost on shutdown):

```sh
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 \
MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 \
DATABASE_URL=sqlite::memory: cargo run --bin maidan-server
```

`MAIDAN_ALLOW_INSECURE_DEV_KEK=1` is the explicit development opt-in to a public
content key-encryption key; without it, or a real `MAIDAN_CONTENT_KEK`, the server
refuses to start ([Production.md](Production.md#crypto-shredding)).
`MAIDAN_SESSION_SECRET` (at least 32 bytes) signs browser sessions and
subscribe-resume tokens. The server refuses to start without it when OIDC is on,
and without OIDC unless `MAIDAN_SUBSCRIBE_RESUME_SECRET` is set instead.
`?mode=rwc` lets SQLite create `dev.db` on first run.

The server detects the dialect from the `DATABASE_URL` prefix
(`postgres://`, `postgresql://`, or `sqlite:`) and selects the
appropriate migration runner and backend automatically.

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

### Helm

`helm/maidan` installs the server against a database and object store you
already run. `helm/maidan-stack` wraps it and can run its own Postgres
(`maidan-postgres`, the pgvector image above) and MinIO (Chainguard's, by the
digest compose uses), wiring the server to both. Values, refusals and the
install commands are in [Production.md](Production.md#kubernetes-with-helm) and
`helm/maidan-stack/README.md`. To try the stack on the kind cluster above:

```sh
helm install demo helm/maidan-stack -f helm/maidan-stack/values-ci.yaml \
  --set maidan.contentKek="$(openssl rand -hex 32)" -n maidan-stack --create-namespace
```

### Required secret keys

| Key                  | Required?            | Notes                                  |
|----------------------|----------------------|----------------------------------------|
| `DATABASE_URL`       | yes                  | Postgres connection string.            |
| `MAIDAN_CONTENT_KEK` | yes                  | 32-byte key-encryption key (`openssl rand -hex 32`); wraps per-message content keys. Keep it out of data backups. |
| `MAIDAN_CONTENT_KEK_PREVIOUS` | during a rotation | Comma-separated retired KEKs still able to unwrap. |
| `MAIDAN_SESSION_SECRET` | yes | At least 32 bytes; signs sessions and subscribe-resume tokens. Must match across replicas. Required with OIDC; without OIDC, `MAIDAN_SUBSCRIBE_RESUME_SECRET` can stand in for it. |
| `S3_ENDPOINT`        | only if S3 backend   |                                        |
| `S3_BUCKET`          | only if S3 backend   |                                        |
| `S3_REGION`          | only if S3 backend   |                                        |
| `S3_ACCESS_KEY_ID`   | only if S3 backend   |                                        |
| `S3_SECRET_ACCESS_KEY` | only if S3 backend |                                        |
| `OTLP_ENDPOINT`      | optional             | OTLP gRPC for traces; metrics when `OTLP_METRICS=1`. |
| `OTLP_METRICS`       | optional             | Set `1` to push `/metrics` instruments via OTLP.   |

`base/secret.example.yaml` documents the contract but contains no real
values.

## Image build matrix

| Image            | Dockerfile                                  | Purpose                |
|------------------|---------------------------------------------|------------------------|
| `maidan-server`  | `crates/maidan-server/Dockerfile`           | Production binary.     |
| `maidan-server`  | `crates/maidan-server/Dockerfile.dev`       | Dev hot-reload.        |
| `maidan-postgres`| `docker/Dockerfile.db`                      | Postgres + pgvector.   |
| `maidan-cli`     | `docker/Dockerfile.cli`                     | The `maidan` CLI, pinned to the server's tag. |
| (local only)     | `docker/Dockerfile.quickstart`              | Built by `compose.quickstart.yaml` from a pinned release binary; not published, not for production. |

## try.maidan.world

try.maidan.world is the public demo, a no-sign-up sandbox. A visitor clicks
"Start a sandbox" and gets a workspace of their own: a `#general` channel, a
"Say hello" task, an admin token for the console and an agent token for an MCP
client, both expiring in 24 hours. The landing's "Try it now, no sign-up" links
here. The Worker and its config are `deploy/cloudflare/src/demo.js` and
`deploy/cloudflare/wrangler.demo.jsonc`.

**The page.** The Worker serves the try page itself from
`deploy/cloudflare/try` (`src/try-page.js`), so a visit never wakes the
container. Its button posts `POST /sandbox`, and the answer fills in the agent
token (shown once), the Claude Code command, "Add to Cursor" and "Add to VS
Code" links, a Codex config and the prompt for the task. "Open the console"
trades the admin token for an HttpOnly session with
`POST /auth/session/from-token`, as the console's own token sign-in does, and
opens `/ui/`. No secret goes into a URL the browser sends anywhere: the Cursor
and VS Code links carry the agent token, but they are `cursor:` and `vscode:`
links that the browser hands to the local app.

**The demo agent.** For a visitor with no MCP client, "Watch an agent do it"
runs a scripted agent in the page (`try/demo-agent.js`) with the sandbox's
agent token. It makes the five `tools/call`s the Say hello task's opening
message asks for, on this origin's `/mcp/streamable`: `claim_next_thread` on
`#general`, `acknowledge_claim`, `post_message`, `set_thread_result` and
`transition_thread` with `start_review`. It sends the token only in the
`Authorization` header and no cookie, so a console session never rides along.
Each step shows in the page with the server's answer, and a refused call
shows the server's own words on its step; "Try again" resumes at that step.
It stops at review and points the visitor to the console, where they approve
the task: the server refuses an agent closing its own work. A run is five
calls, far inside the per-client and per-sandbox limits below, and the page's
CSP (`connect-src 'self'`) already covers it.

**The theme.** The page wears the console's graphite palette and is dark by
default, with the landing's warm paper in light. It loads `/theme.js`, the
console's own pre-paint script (`web/console/public/theme.js`), which the
deploy copies into the page's assets, so a visitor who chose Light in the
console sees the light page. The console's `/ui/theme.js` is the container's,
and loading it would wake the container on every visit.

**The mint.** `POST /sandbox` (`crates/maidan-server/src/sandbox.rs`) is on only
with `MAIDAN_DEMO_SANDBOX=1`, and under `MAIDAN_ENV=production` only beside
`MAIDAN_DEMO_INSTANCE=1`, the marker that the whole instance is a throwaway.
Boot refuses it beside `MAIDAN_SIGNUP`, or without
`MAIDAN_PUBLIC_ORIGIN`. Elsewhere the route answers 404. Each sandbox is an
ordinary tenant, so one visitor's tokens see nothing of another's
(`demo_sandbox_e2e`). What bounds it:

| Limit | Value on try.maidan.world |
| --- | --- |
| Sandboxes per address | 3 an hour (`MAIDAN_DEMO_SANDBOX_RATE_LIMIT_MAX`) |
| Sandboxes in the instance | 200 (`MAIDAN_DEMO_SANDBOX_MAX`), then 503 until the reset |
| Per sandbox | a hard cutoff at 5000 requests, 2000 MCP calls and 4 MiB stored: each sandbox is an `open` workspace, so the free allowance refuses past it ([Operations](Operations.md)) |
| Request rate | 60 a minute per client, 120 a minute per workspace |
| Bodies | 64 KiB, refused at the edge before they wake the container |
| Human check | Turnstile, when the Worker has `DEMO_TURNSTILE_SITE_KEY` and the secret `DEMO_TURNSTILE_SECRET`. The Worker checks it, since the container cannot reach Cloudflare |
| Cross-site | a browser post from another origin is refused |
| Advice from a decision model | About 10 questions a minute across the demo (`JEV_LIMIT` in `wrangler.demo.jsonc`; Cloudflare's limiter is approximate and allows short bursts), each at most 16 KiB, the server's own cap on a request to the provider. At that rate a day of the largest requests costs under $10, and a hand-off is usually about a thousand tokens; it is a design figure, not a spending cap. On only when the Worker has the secret `TYPESAFE_API_KEY` |

**Images.** The demo runs the stock `maidan-server:<tag>`, which has no
bootstrap routes. No release builds an image with them:
`scripts/check-deploy-contract.sh` (CI `deployment contract`) fails if a
workflow builds with the bootstrap feature or a release leg builds a second
server image.

**Deploy.** `.github/workflows/deploy-demo.yml` runs when `release.yml`
dispatches it for the tag it just published, or on a dispatch with a tag.
A release created with the workflow's `GITHUB_TOKEN` does not trigger
`release: published`, so that event never starts this workflow on its own.
It does nothing unless the repository variable
`DEMO_DOMAIN` is `true`. It checks that the tag is on `main` and cosign-verifies
`maidan-server:<tag>`. It then builds `deploy/cloudflare/demo/Dockerfile`,
which copies the binary by digest onto Debian with bash and openssl and
compiles nothing. It pushes the image with `wrangler containers push` and
deploys `wrangler.demo.jsonc` with the try page as the Worker's static assets,
as `deploy-prod.yml` does for maidan.world. It reads `CLOUDFLARE_API_TOKEN` and
`CLOUDFLARE_ACCOUNT_ID` from the repository secrets. The container's keys are
made fresh on each boot, so the demo stores no secret. The route's
`custom_domain` creates the DNS record for try.maidan.world on the first
deploy.

**Turning it on.** Once a release carries `POST /sandbox`:

1. Set the repository variable `DEMO_DOMAIN` to `true`
   (`gh variable set DEMO_DOMAIN --body true`). `CLOUDFLARE_API_TOKEN` and
   `CLOUDFLARE_ACCOUNT_ID` are the secrets `deploy-prod.yml` already uses.
2. Publish a release, or dispatch `deploy demo` with its tag. The deploy
   creates the try.maidan.world DNS record (`custom_domain`); the zone must be
   on the same Cloudflare account.
3. Open `https://try.maidan.world`, start a sandbox, and connect an agent.
4. Recommended: make a Turnstile widget for `try.maidan.world`, store its
   secret with
   `npx wrangler secret put DEMO_TURNSTILE_SECRET -c wrangler.demo.jsonc`,
   and add `DEMO_TURNSTILE_SITE_KEY` to the `vars` in `wrangler.demo.jsonc`.
   The next deploy turns the check on; with only one of the two set, it stays
   off.
5. Optional: advice from a decision model on each sandbox's tasks
   ([Decision models](Decision-Models.md)). Store a TypeSafe API key with
   `npx wrangler secret put TYPESAFE_API_KEY -c wrangler.demo.jsonc`. The
   container still has no internet: its server asks `http://jev.internal/`,
   the Worker forwards that one call to TypeSafe with the key
   (`src/demo-jev.js`), and the key never reaches the container. The next
   request after the secret is set recycles the container with advice on.

**Boot and reset.** On every boot `demo/entrypoint.sh` deletes any old
database, makes this boot's keys and starts the server; nothing is seeded.
Once a day at 04:00 UTC the cron destroys a running container, which deletes
every sandbox. It never starts a stopped one, so the next visitor boots an
empty demo.

**Cost.** One `basic` instance (`max_instances: 1`) sleeps after 15 idle
minutes. The try page is answered by the Worker, so only a sandbox and the
agents using it wake the container. The cron runs once a day and only
destroys. Nothing else runs on a timer. `deploy/cloudflare/test/cost.test.js`
holds all of this.

## Migrations

`maidan-server` is the **single source of truth** for schema; it applies
all pending migrations on boot via `run_postgres_migrations` (or
`run_sqlite_migrations`, depending on the dialect). The
`maidan-postgres` image is a thin pgvector layer that does **not**
bundle schema into `docker-entrypoint-initdb.d` — fresh volumes and
upgrades go through the same code path.

A rollout that keeps the previous binary serving while the new one migrates
follows [Migrations](Migrations.md). The migration that runs in that window
only expands. A drop or a rewrite waits for a later contract. A migration
that cannot expand is a cutover: stop the previous binary first.
