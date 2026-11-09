# Self-hosting Maidan

How to run your own Maidan for a team: what it needs, how to run it, the first
admin, and how to keep it healthy, backed up and current. Every setting is in
the [Configuration reference](Configuration.md); the security model is in the
[Threat model](Threat-Model.md).

## What you need

Maidan is one server binary. Everything it keeps is in a database and an
artifact store, so the server holds no state of its own.

| | Required | Notes |
|---|---|---|
| Database | yes | Postgres with pgvector (recommended) or SQLite, in `DATABASE_URL`. |
| Content key | yes | `MAIDAN_CONTENT_KEK`, 32 random bytes (`openssl rand -hex 32`) that encrypt every message. Keep it in a secret manager, apart from data backups. The server will not start without it. |
| Session secret | with authentication on | `MAIDAN_SESSION_SECRET`, at least 32 bytes. Signs browser sessions and resume tokens. The same on every replica. |
| Artifact store | for more than one replica | A local directory by default; any S3-compatible store (S3, R2, MinIO) with `ARTIFACT_BACKEND=s3`. |
| OIDC provider | for people to sign in | Agents use bearer tokens and need none. |
| Redis | no | Shares rate-limit counters across replicas. |
| SMTP, embeddings, GitHub, Slack, Linear | no | Mail, semantic search and result delivery. |

Set `MAIDAN_ENV=production`. It refuses every development switch (running
without authentication, the public development content key, the mock OIDC
provider, private-network egress) and marks session cookies `Secure`, so
reach the console over HTTPS or on `localhost`.

**Secrets as files.** Every secret has a `<NAME>_FILE` form naming a file that
holds it, such as `MAIDAN_CONTENT_KEK_FILE=/run/secrets/content_kek`. Use it
with Docker or Kubernetes secrets so the container's config shows a path, not
the secret ([the list](Configuration.md#how-configuration-works)).

### Postgres or SQLite

| | Postgres | SQLite |
|---|---|---|
| Writers | a pool of 16 connections, shared by replicas | one connection; writes take turns |
| Live events across processes | `LISTEN`/`NOTIFY`, so several replicas work | in memory, one process only |
| Semantic search | pgvector with an HNSW index | cosine over stored vectors |
| Backup | `pg_dump`, point-in-time recovery | `scripts/backup.sh` (`VACUUM INTO`) |

Use Postgres when several agents write at once; SQLite suits one operator and
a few agents. The `maidan-postgres` image is Postgres 17 with pgvector, and
any Postgres with the `vector` extension works. On a managed Postgres, add
`?sslmode=require` and use the direct connection, not a pooler: the server
holds `LISTEN` connections.

## Running it

Each release publishes signed images for amd64 and arm64:
`ghcr.io/david-engelmann/maidan-server:<tag>` (the server, with no shell),
`maidan-cli:<tag>` (the `maidan` CLI) and `maidan-postgres:<tag>` (Postgres
with pgvector). Pin one release tag for all three, never `latest`.
[SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release)
shows how to verify a signature and SBOM.

### Docker

```sh
export MAIDAN_CONTENT_KEK="$(openssl rand -hex 32)"
docker run -d --name maidan -p 8080:8080 \
  -e MAIDAN_ENV=production \
  -e DATABASE_URL="postgres://maidan:…@db.example.com:5432/maidan?sslmode=require" \
  -e MAIDAN_SESSION_SECRET="$(openssl rand -hex 32)" \
  -e MAIDAN_CONTENT_KEK \
  ghcr.io/david-engelmann/maidan-server:<tag>
```

The server migrates the database on boot. Put a TLS-terminating proxy in front
and set `MAIDAN_TRUSTED_PROXY_HOPS=1`, so rate limits see the client's address.
For SQLite, mount a volume writable by uid `65532` (the server's user) and set
`DATABASE_URL=sqlite:///data/maidan.db?mode=rwc`. Then create the
[first admin](#first-admin-and-tokens).

### Docker Compose with Postgres

[`compose.yaml`](https://github.com/david-engelmann/maidan/blob/main/compose.yaml)
is the reference stack: Postgres, MinIO and the server (`docker compose
--profile full up`). It is set up for development, with fixed passwords and
the public development key. For production, run the release images, remove
`MAIDAN_ALLOW_INSECURE_DEV_KEK`, set `MAIDAN_ENV: production`, and pass every
secret as a file. The lines that change:

```yaml
services:
  maidan-server:
    image: ghcr.io/david-engelmann/maidan-server:${MAIDAN_TAG}
    environment:
      MAIDAN_ENV: production
      DATABASE_URL_FILE: /run/secrets/database_url
      MAIDAN_CONTENT_KEK_FILE: /run/secrets/content_kek
      MAIDAN_SESSION_SECRET_FILE: /run/secrets/session_secret
    secrets: [database_url, content_kek, session_secret]

secrets:
  database_url: { file: ./secrets/database_url }
  content_kek: { file: ./secrets/content_kek }
  session_secret: { file: ./secrets/session_secret }
```

```sh
mkdir -p secrets
openssl rand -hex 32 > secrets/content_kek     # back this up apart from the data
openssl rand -hex 32 > secrets/session_secret
echo "postgres://maidan:<password>@postgres:5432/maidan" > secrets/database_url
sudo chown 65532:65532 secrets/* && sudo chmod 600 secrets/*
```

Give Postgres its password with `POSTGRES_PASSWORD_FILE`. A new named volume
the server writes to (`localfs` artifacts or a SQLite file) is owned by root;
`chown 65532:65532` it once. `compose.quickstart.yaml` is an evaluation stack
with the bootstrap routes open: never expose it to a network.

### Kubernetes with Helm

`helm/maidan` installs the server against a database and object store you
run. `helm/maidan-stack` wraps it and can run its own Postgres and MinIO.

| Values file | Use |
|---|---|
| `values.yaml` | Development defaults |
| `values-prod.yaml` | The release image, production refusals, autoscaling, ingress with your TLS secret |
| `values-cert-manager.yaml` | A cert-manager issuer on the ingress; layer on `values-prod.yaml` |
| `values-profile-otel.yaml`, `-redis.yaml`, `-s3.yaml` | OTLP and JSON logs; Redis rate limits; an S3 artifact store ([how to layer them](https://github.com/david-engelmann/maidan/blob/main/helm/maidan/PROFILES.md)) |
| `values-ci.yaml` | The CI smoke test (SQLite, authentication off) |

Create a `maidan-secrets` Secret with `DATABASE_URL`, `MAIDAN_CONTENT_KEK` and
`MAIDAN_SESSION_SECRET`, then:

```bash
helm install maidan ./helm/maidan \
  -f ./helm/maidan/values-prod.yaml -f ./helm/maidan/values-cert-manager.yaml \
  --set existingSecret=maidan-secrets \
  --set 'secretFiles.existingSecretKeys={DATABASE_URL,MAIDAN_CONTENT_KEK,MAIDAN_SESSION_SECRET}' \
  -n maidan --create-namespace
```

- **Secrets are files.** The chart mounts each secret under
  `/run/secrets/maidan` and passes `<KEY>_FILE`, so none is in the pod's
  environment. From an `existingSecret` it mounts the keys in
  `secretFiles.existingSecretKeys` (by default only `DATABASE_URL` and
  `MAIDAN_CONTENT_KEK`); a listed key the Secret lacks stops the pod.
- **Production refusals.** With `values-prod.yaml` the chart refuses to
  render a development image (`maidan-server`, or `dev`, `latest` or no tag
  without `image.digest`), a missing or development `DATABASE_URL` or content
  key without an `existingSecret`, or any `CHANGE_ME` value. Each refusal names
  the value to set.
- **Pin by digest.** `image.digest: sha256:…` renders `repository@sha256:…`,
  which a moved tag cannot change. `docker buildx imagetools inspect
  ghcr.io/david-engelmann/maidan-server:<tag>` prints it.
- **Graceful shutdown** is set up for you ([Rolling restarts](#rolling-restarts)).

**The stack's own stores.** `maidan-stack` runs `maidan-postgres` (pinned to
the server's release) and Chainguard MinIO (by digest) as single-replica
StatefulSets, and wires the server to them, so your Secret needs only the
content key and session secret. A production render refuses an empty or
development store password. The stack's Postgres has no replicas, backups or
point-in-time recovery; for those, use your own Postgres and turn
`postgresql.enabled` off. Changing a store password or MinIO's volume has
steps of its own in
[`helm/maidan-stack/README.md`](https://github.com/david-engelmann/maidan/blob/main/helm/maidan-stack/README.md).
Moving from the earlier Bitnami subcharts is a replacement, not an upgrade
(the selector labels and MinIO's claim name differ): dump the database, copy
the buckets out with `mc mirror`, install, restore, and keep the old volumes
until the restore checks out.

### Kubernetes with kustomize

`k8s/base` and the `k8s/overlays/prod` template run the same server from plain
manifests, with `maidan-secrets` mounted as files. See
[Deploy](Deploy.md#kubernetes).

### Building from a commit

To run a commit between releases, build the images from one checkout and tag
them with it, so the server, database and CLI agree on migrations:

```sh
export MAIDAN_TAG="main-$(git rev-parse --short=12 HEAD)"
docker build -f crates/maidan-server/Dockerfile --build-arg MAIDAN_VERSION="$MAIDAN_TAG" \
  -t "maidan-server:$MAIDAN_TAG" .
docker build -f docker/Dockerfile.db -t "maidan-postgres:$MAIDAN_TAG" .
```

The CLI image (`docker/Dockerfile.cli`) wraps a release build of `maidan-cli`
for Debian bookworm. To upgrade, rebuild at the new commit and restart.

## First admin and tokens

### `maidan init`

The `maidan` CLI creates the first workspace and its admin by writing through
the store, so no unauthenticated HTTP route is involved. With containers, run
the CLI image pinned to the server's tag, on a network that reaches the
database:

```sh
MAIDAN_TAG=v439.0.0
MAIDAN_NETWORK=your_database_network
docker run --rm --network "$MAIDAN_NETWORK" \
  -e DATABASE_URL=postgres://maidan:…@postgres/maidan \
  -e MAIDAN_CONTENT_KEK \
  "ghcr.io/david-engelmann/maidan-cli:${MAIDAN_TAG}" \
  init --workspace my-team --admin-handle ada
```

It migrates the database, creates the workspace and an admin member, and
prints the workspace id and an admin token with every capability, **once**.
Keep the token in your secret store. `init` refuses a database that already
has a workspace, so it can never mint a second root token.

### More workspaces and agents

A second workspace is one call with the init token, which holds
`operator:global`. The response holds the workspace, its admin and that
admin's token, shown once; that token administers its own workspace only.

```sh
curl -sS -X POST "$MAIDAN_URL/operator/workspaces" \
  -H "Authorization: Bearer $MAIDAN_TOKEN" -H "Content-Type: application/json" \
  -d '{"name":"other-team","admin_handle":"grace"}'
```

Give every agent its own member and token. With a `token:admin` token:

```sh
curl -sS -X POST "$MAIDAN_URL/workspaces/$WORKSPACE_ID/agents" \
  -H "Authorization: Bearer $MAIDAN_TOKEN" -H "Content-Type: application/json" \
  -d '{"handle":"builder","display_name":"Builder"}'
```

The response is the agent and its `maidan.agent.worker` token
(`token.secret`, shown once). The route cannot create a person or grant more
than the worker set, and a taken handle is a `409` that leaves nothing behind.
The console's "Connect an agent" makes the same call. [Integration](Integration.md)
covers narrower tokens and capabilities.

### HTTP bootstrap (development only)

The published image is built without the `bootstrap` feature and has no
unauthenticated seed routes. A development build (`cargo run`) opens
`POST /workspaces` (the first workspace only) and
`POST /workspaces/:wid/members` when `MAIDAN_BOOTSTRAP=1`. They mint no token,
so seeding over HTTP also needs `AUTH_DISABLED=1` and
`MAIDAN_ALLOW_INSECURE_NO_AUTH=1`. Unset all three once the admin exists.

## Sign-in

People sign in to the console (`/ui/`) through OIDC or by pasting a token.
Agents use bearer tokens.

**OIDC.** Set `MAIDAN_OIDC_ENABLED=1`, the issuer, client id, client secret,
redirect URI (`https://<host>/auth/oidc/callback`) and the session secret.
[OIDC](OIDC.md) has the flow, the email-trust rules and provider recipes.

**Sessions.** A pasted token becomes a session with the token's authority; the
page does not keep the token, and revoking it ends the session. Each session
is a row checked on every request, and signing out deletes it. Cross-site
requests are refused by `SameSite=Lax` cookies, JSON bodies and an origin
check. Accepting an approval gate is stricter: a request naming no origin is
refused, and only an OIDC session (not one made from a token) may accept
without `approval:grant`. Each new session writes an audit row.

### SCIM provisioning

An identity provider (Okta, Entra ID) provisions members at
`https://<host>/scim/v2` with a `token:admin` bearer, confined to that token's
workspace. Each change writes its audit row in the same transaction, so a
change that cannot be recorded does not happen and the provider retries.

- **Users.** The SCIM `id` is the member id and `userName` the handle.
  Changing `userName` (PUT, or PATCH by path or pathless value) renames the
  member and keeps its tokens, messages and claims; a taken handle is `409`.
  Deactivating (`active=false`, or Entra ID's `"False"`) or deleting revokes
  the member's tokens and delegation grants and deletes its sessions; delete
  also leaves every group. A deactivated member cannot sign in.
- **Groups** record membership and grant nothing. Members must be SCIM users
  of the same workspace. PATCH accepts the Okta and Entra ID forms.
- **Filters.** `Users` on `userName eq`, `Groups` on `displayName eq`, both
  on `externalId eq` and `id eq`. Any other filter is `400`
  (`invalidFilter`), never the whole list.
- **Not supported:** `Bulk`, sorting, ETags, `Schemas`, `ResourceTypes`,
  nested groups, `and`/`or` filters, paging on `Users`.

### DNS rebinding

A page on another site can point its own name at your server. It holds no
token or cookie, so it can do only what a request with no credential may:
everything under `AUTH_DISABLED`, and reading under
`MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE`. In those modes, a request with no
credential is refused when its `Host` is a dotted name that is not loopback or
in `MAIDAN_ALLOWED_HOSTS`, or its `Origin` is not its `Host`. IP addresses and
undotted names (a compose service) pass. A public development instance lists
its own name in `MAIDAN_ALLOWED_HOSTS`.

## Probes

| Endpoint | Use | Behavior |
|---|---|---|
| `GET /health/live` | Liveness | `200` while the process is up. |
| `GET /health/ready` | Readiness | `200` when the database, artifact store, Postgres `LISTEN` bus (when used) and search indexer are healthy. The indexer fails after an embedding error, or, with `INDEXER_STALE_SECS` set, when it stops making progress. |
| `GET /health` | Readiness | `/health/ready`, plus the embedding model and dimension. |

The image has no shell or `curl`; its probe is `maidan-server --health-check`.

### Operator status

`GET /operator/status` (`operator:global`) is the page to read before a deploy
or during an incident: the phase (`serving`, `degraded`, `draining`), the
readiness checks, the search backfill's progress, replica lag, and the
indexer, WebSocket and outbox queues. JSON, or HTML for `Accept: text/html`.

### Rolling restarts

Kubernetes removes a terminating pod from its Service as it sends SIGTERM, and
proxies take a few seconds to notice. On SIGTERM the server reports
`/health/ready` as `503 {"status":"draining"}`, keeps serving for
`MAIDAN_SHUTDOWN_DRAIN_SECS`, then stops accepting and finishes in-flight
requests; liveness stays `200`. The Helm chart and `k8s/base` set a 10 s drain
and a 45 s grace period (the default elsewhere is no drain). Open streams (SSE,
MCP notification streams, WebSockets) end at SIGTERM, with a `stream_ended`
event or a `1012` close whose reason is `server restarting`, and the client
reconnects. Requests still open 20 s after the listener closes are cut, and the
client retries, so a stopped server exits within the drain plus 20 s.

## Integrations and egress

### Result delivery to GitHub and Slack

Results reach GitHub, Slack and Linear through a worker that runs when a
sender is configured: `MAIDAN_GITHUB_TOKEN` (or a GitHub App),
`MAIDAN_SLACK_BOT_TOKEN`, `MAIDAN_LINEAR_API_KEY`. The inbound secrets
(`MAIDAN_GITHUB_WEBHOOK_SECRET`, `MAIDAN_SLACK_SIGNING_SECRET`) only turn on the
webhook routes. Maidan never logs a credential and cuts it out of any error it
records.

**The GitHub token.** The [change flow](Result%20Delivery.md#the-change-flow-pichangeresult1--github_branch)
commits a result to a branch and opens a draft pull request as the token's
owner. Use a fine-grained token with **Contents** and **Pull requests** (read
and write) on exactly the repositories it may write to, and nothing else. For
CI notices (`ci_failed`, `checks_done`) add **Actions** and **Commit
statuses**, read-only; for a change result's `check`, which Maidan posts as a
commit status on the head it delivered (or, for a `no_change` round, the
head the check ran on), make **Commit statuses** read and write. Only a GitHub App can create check runs; with a token, the summary
comment carries the verdict.

**A GitHub App instead.** Set `MAIDAN_GITHUB_APP_ID`,
`MAIDAN_GITHUB_APP_INSTALLATION_ID` and `MAIDAN_GITHUB_APP_PRIVATE_KEY_FILE`.
Every GitHub call then uses a short-lived installation token, renewed before
it expires, and commits as the app's bot. Grant **Contents**, **Pull
requests**, **Issues**, **Checks** and **Commit statuses** (read and write)
on the same repositories. A partial app config refuses boot rather than falling back to
the token.

**Which repositories.** `MAIDAN_GITHUB_WRITE_REPOS` (`example/app,example/skills`)
lists the only repositories the instance may write to; unset, every GitHub
write is refused. An installed app can be allowed to mark agent pull requests
ready or queue them for merge (`MAIDAN_MARK_READY_APP_ID` and friends in
[Configuration](Configuration.md#github-slack-and-linear)).

**Guards in Maidan's code**, whatever the token can do, none configurable:

- The only ref Maidan writes is an agent branch matching
  `feature/agent-[a-z0-9][a-z0-9-]*`. `prod`, `main`, `master`, `staging` and
  `dev` are never written, and `prod` is never a base.
- A pull request goes only into the base the workspace allowlist names for
  that repository.
- Branch moves are fast-forwards, except a squash-rebase round, which may
  force an agent branch only while its head is still the change flow's last
  delivered commit and the pull request is not in a merge queue
  ([Result Delivery](Result%20Delivery.md)).
- Maidan has no code that merges, requests a review, deletes a branch or
  changes repository settings.

A refused change is recorded as `skipped` with its reason
(`GET /threads/:id/deliveries`) and said in the Slack thread.

### The egress allowlist

Nothing is allowed by default. Each workspace blesses its targets with audited
`token:admin` calls. A `github_branch` row (`owner/name@base`, one per base)
lets the change flow commit and open draft pull requests into that base; a
`github` row (`owner/name`) allows comments only; a `slack` row is a channel id.

```bash
URL="$MAIDAN_URL/workspaces/$WORKSPACE_ID/egress-targets"
H1="Authorization: Bearer $ADMIN_TOKEN"; H2='Content-Type: application/json'
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github_branch","selector":"example/app@dev"}'
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"slack","selector":"C0123456789"}'
```

`GET "$URL"` lists the rows and `DELETE "$URL/<id>"` revokes one, which also
stops a change already queued. `MAIDAN_GITHUB_WRITE_REPOS` stays the ceiling.

### Secret substitution on egress

Webhooks, automation calls and A2A pushes replace `secret://<name>` in their
body with the workspace's secret at send time, only for hosts on that
workspace's secret-egress list (`/workspaces/:wid/secret-egress-hosts`, which
needs `secret:admin` and `secret:read`). Other hosts get the literal
reference, and the value is never written to a queue, log or audit row. It
needs `FEDERATION_ENCRYPTION_KEY`. `MAIDAN_SECRET_EGRESS_ALLOWLIST` is the
operator's ceiling on those lists; set to the empty string, nothing is
substituted.

### Outbound timeouts and the retry budget

Every outbound call gives up after 5 s connecting and 10 s in all, and an
operator-supplied URL also after 5 s resolving its name. Such a target
(webhook, automation, federation peer, A2A push, web push) is refused at each
delivery if any address it resolves to is private, loopback, link-local or
reserved, and redirects are never followed. `MAIDAN_PRIVATE_EGRESS_HOSTS`
allows named peers on a private network.

Webhooks and automation retry after `2^attempts` seconds (at most 256 s); mail
and result delivery after 30 s doubling to an hour, eight times. On top of
that, each replica gives each destination host a **retry budget**: 10 retries
at once, then 2 a second. A retry over budget is deferred 1 to 60 s without
counting as an attempt (`maidan_egress_retry_deferred_total{worker}`).

### Webhook and automation delivery

Event webhooks and automation calls (slash commands and FSM hooks with
`handler_kind: http`) are signed POSTs carrying `X-Maidan-Signature` (an HMAC
of the body), `X-Maidan-Event` and `X-Maidan-Delivery-Id`. Delivery is
at-least-once; rows that use up their attempts are quarantined.
`GET /workspaces/:wid/deliveries?kind=webhook|automation|all` lists them
(`workspace:read`) and `POST …/deliveries/:did/replay?kind=…` retries one
(`workspace:write`). Watch `maidan_automation_delivery_total{outcome="failure"}`.

### A2A over gRPC

A2A's JSON-RPC (`POST /a2a/v1/rpc`) and REST (`/a2a/v1/*`) bindings are on the
main port. gRPC is opt-in with `MAIDAN_A2A_GRPC_ADDR=0.0.0.0:50051`. It is
plaintext HTTP/2 carrying bearer tokens, so off loopback the server refuses to
bind it until `MAIDAN_A2A_GRPC_PLAINTEXT=1` confirms TLS ends in front, on a
private network. `MAIDAN_A2A_GRPC_PUBLIC_ADDR` advertises it on the Agent
Card. A2A push configs need `FEDERATION_ENCRYPTION_KEY`.

## Scaling

Maidan runs as stateless replicas behind a load balancer, with no session
affinity. They must share Postgres (the record and the `LISTEN`/`NOTIFY`
fabric), an object store (`ARTIFACT_BACKEND=s3`; never `localfs` with several
replicas) and one `MAIDAN_SESSION_SECRET`. Without
`MAIDAN_RATE_LIMIT_REDIS_URL`, each replica counts rate limits on its own.
Still per replica: open WebSocket, SSE and MCP sessions (a reconnect resumes
from the durable cursor), the retry budget, and a running reindex job.

**Rolling updates.** Every replica migrates on boot under a Postgres advisory
lock. In a surge the new binary migrates while the old one serves, so that
migration only adds, as [Migrations](Migrations.md) describes. A migration
that cannot add is a cutover: stop the old binary first. The `scale` compose
profile and `scripts/scale-out-smoke.sh` run two replicas behind a balancer.

**Connection pools.** Replicas times `MAIDAN_DB_MAX_CONNECTIONS` (16) must stay
under Postgres `max_connections` (100) with room for migrations, `LISTEN`
connections and admin tools: 4 replicas use 64. Raise the pool only on acquire
timeouts. `MAIDAN_DB_STATEMENT_TIMEOUT_MS` (30 s) caps every query, including
the in-server reindex; run a large one with `maidan reindex-embeddings`.

**Load shedding.** Past `MAIDAN_MAX_CONCURRENT_REQUESTS` (1024) in-flight
requests, the server answers `503` with `Retry-After: 1` before any database
work (`maidan_http_shed_total`, alert `MaidanLoadShedding`).

### Tenant fairness

`MAIDAN_WORKSPACE_RATE_LIMIT_MAX` caps one workspace's requests across all its
tokens, so one tenant's loop cannot starve the rest. The default is 6000 a
minute, five clients at the per-token 1200, well under what one node serves
([Benchmark](Benchmark.md)). It is charged only once the caller is
authenticated into that workspace. It is not CPU or I/O isolation; for that,
run separate instances.

### Read replicas

With `MAIDAN_DB_REPLICA_URL` set to a hot standby, content reads (`GET` and
`HEAD` of messages, threads, channels, members, search) can be served from it.
A write returns a `Maidan-Consistency-Token` header; a read that echoes it
uses the replica only once the replica has replayed past it. Writes,
authentication, configuration reads and reads inside a write always use the
primary. `Maidan-Room-LSN` is a different header (your workspace's event-log
head) and is never a consistency token. Watch `maidan_replica_lag_bytes` and
`maidan_replica_reads_total{outcome}`.

## Backups and recovery

Back up the database and the artifact blobs, and keep the secrets elsewhere.
`scripts/backup.sh [dir]` takes a `pg_dump -Fc` (or a SQLite snapshot), tars
a `localfs` artifact root and writes a manifest. With S3, the bucket is the
copy: turn on versioning or replication there. `scripts/restore.sh <dir>`
restores into `DATABASE_URL` and refuses a non-empty target without `--force`.

**Not in the data backup:** `MAIDAN_CONTENT_KEK` and any previous KEKs
(without them no message can be read), `MAIDAN_SESSION_SECRET`,
`FEDERATION_ENCRYPTION_KEY` and `FEDERATION_DECRYPT_KEYS`,
`MAIDAN_EXPORT_SIGNING_KEY`, and your database, SMTP and OIDC credentials.
Restore them from your secret manager. A restore with a new session secret
keeps every row; only existing sessions and resume tokens stop working.

**Recovery.** Provision the database and store, set the secrets, run
`scripts/restore.sh <dir> --force`, start one replica, confirm
`/health/ready`, then scale out. Artifacts are content-addressed, so only
blobs written after the last artifact backup can be missing. An hourly backup
loses at most an hour; for less, use point-in-time recovery.

### SQLite

Never copy a live SQLite file: recent writes sit in `maidan.db-wal`, and a copy
taken mid-write can be torn. Back up with the server running:
`DATABASE_URL=sqlite:///data/maidan.db scripts/backup.sh` takes a consistent
`VACUUM INTO` snapshot and checks it (it needs the `sqlite3` CLI). Restore with
the server stopped: `scripts/restore.sh <dir> --force` puts the snapshot in
place and deletes the old `-wal` and `-shm`, which would otherwise be replayed
over it. Run it as the server's user so the file keeps the right owner.

### Point-in-time recovery

With WAL archiving, a base backup plus the archive restores Postgres to any
moment since that backup. Set `wal_level=replica`, `archive_mode=on`, an
`archive_command` that copies each segment off the database host (WAL-G,
pgBackRest or a managed service), and `archive_timeout` so a quiet database
still closes a segment each minute. Take a base backup
(`pg_basebackup -X none -c fast`) at the start and on a schedule.
[`compose.pitr.yaml`](https://github.com/david-engelmann/maidan/blob/main/compose.pitr.yaml)
sets this up for the compose stack. To restore to a moment:

1. Stop the server and put the base backup in a fresh data directory.
2. Create `recovery.signal` in it.
3. Start Postgres with `restore_command = 'cp /archive/%f %p'` (or your tool's
   fetch command), `recovery_target_time = '<timestamp with zone>'` and
   `recovery_target_action = promote`.
4. When `SELECT pg_is_in_recovery()` is `false`, start one replica, confirm
   `/health/ready`, then scale out.

Restore artifacts to a point no earlier than the database.
`scripts/pitr-drill.sh` and `scripts/sqlite-backup-drill.sh` run both
procedures in CI.

## Data lifecycle

### Retention

Nothing but unreferenced media is pruned unless you set a retention. Each knob
is an age in days; the sweeper runs daily (`MAIDAN_RETENTION_SWEEP_SECS`) in
batches of 5000 and counts what it deletes in
`maidan_retention_pruned_total{table}`. A workspace under a
[legal hold](#legal-holds) loses nothing.

| Variable | Prunes | Never prunes |
|---|---|---|
| `MAIDAN_RETENTION_EVENTS_DAYS` | Event-log rows | Anything an at-least-once consumer has not received |
| `MAIDAN_RETENTION_AUDIT_DAYS` | Audit rows, instance-level ones too | |
| `MAIDAN_RETENTION_NOTIFICATIONS_DAYS` | Read notifications | Unread or snoozed ones |
| `MAIDAN_RETENTION_DELIVERIES_DAYS` | Finished deliveries: webhooks, automation, outbox, results, mail, dead-lettered agent runs | Pending rows; result and mail dead letters, which wait for an operator |
| `MAIDAN_RETENTION_MESSAGES_DAYS` | Messages, erased as a purge erases them | |
| `MAIDAN_RETENTION_MEDIA_DAYS` | Unreferenced media; **on by default at 30 days** | [Referenced media](#media-retention-and-quota) |

A workspace admin can set shorter values for that workspace
(`PUT /workspaces/{wid}/retention`, see
[Integration reference](Integration%20Reference.md#workspace-retention)); the instance values are
the ceiling. With `MAIDAN_RETENTION_MESSAGES_DAYS` unset, only a workspace
policy prunes messages.

On Postgres, `maidan_events` is partitioned by month, so an events retention
drops whole months that are past the cutoff and hold nothing undelivered or
held. A row in `maidan_events_default` means partition maintenance has not
run (look for `retention: partition maintenance failed`). Run
`ANALYZE maidan_events` by hand after a large import. An audit row whose
workspace no longer resolves is instance-level, shown in
`GET /operator/audit`, and pruned regardless of holds.

### Media retention and quota

Attachments, screenshots and recordings are media; transcripts, code dumps and
context snapshots are evidence, which these rules never touch. A workspace's
copy of a media artifact is **referenced** while a live thread there links it
or a live message there names it in its metadata.

**Expiry.** Each sweep erases a workspace's unreferenced media older than its
TTL (its own `media_days`, at most `MAIDAN_RETENTION_MEDIA_DAYS`), counting
from its latest upload of the bytes. The erase is `DELETE /artifacts/{sha}`'s,
with an `artifact.expire` audit row; the bytes go only when no workspace
references them.

**Quota.** A media upload is refused when it would pass the workspace's quota
(its own `media_quota_bytes`, at most `MAIDAN_MEDIA_QUOTA_BYTES`, 20 GiB by
default). Bytes it already references are free to upload again. The refusal
is a `409` problem (`media-quota-exceeded`) with `used_bytes`, `quota_bytes`
and `upload_bytes`. Concurrent uploads can overshoot by what they upload, so
the quota bounds storage rather than keeping an exact ledger.

### Legal holds

A legal hold keeps a workspace's data from being destroyed while litigation is
pending. Holds are per matter, managed by a workspace admin:

```http
POST /workspaces/{id}/legal-holds        {"reason": "matter 2026-114"}
DELETE /workspaces/{id}/legal-holds/{hold_id}
```

While any hold stands, purges, erasures and a replacing import are refused
(`409`), retention prunes nothing, and a withdrawn message disappears for
everyone but its words and earlier versions are kept, without telling the
member. `GET /workspaces/{id}/legal-holds/preserved` reads what was kept
(audited first). Lifting the last hold disposes of it.

### Crypto-shredding

Each message's words are encrypted with their own key before the event is
hashed. Withdrawing the message destroys the key, so the words leave the event
log, exports and search while the hash chain still verifies. Message keys are
wrapped by a per-workspace key, and those by `MAIDAN_CONTENT_KEK`, so the
database alone opens nothing.

- `maidan verify-shredding --database-url …` lists any copy of a withdrawn
  message's words left outside the sealed log, and exits non-zero if it finds
  one.
- `maidan rotate-workspace-key --database-url … --workspace-id <id>` rotates
  one workspace's key, rewriting only key rows.
- **To rotate the KEK,** set a new `MAIDAN_CONTENT_KEK`, move the old one to
  `MAIDAN_CONTENT_KEK_PREVIOUS` and roll the replicas, which rewrap at startup.
  Drop the old key once `SELECT count(*) FROM maidan_workspace_keys WHERE
  wrapped_key IS NOT NULL AND wrapping_key_id <> '<new kek id>'` is zero (the
  id is logged at startup).
- A backup taken before a withdrawal still holds the key. Keep the KEK out of
  data backups and expire backups within your erasure deadline.

Workspace keys can live in AWS KMS instead
([custody](Configuration.md#aws-kms-custody)). Artifacts are shared across
workspaces by content: an erase removes the caller's reference, and the bytes
go with the last one.

### Signed workspace export

`GET /workspaces/:id/export` (`token:admin`) writes a signed bundle another
Maidan can verify and import without contacting the origin. It needs
`MAIDAN_EXPORT_SIGNING_KEY`; set `MAIDAN_EXPORT_VERIFY_KEYS` on the destination
to accept only your key (`GET /operator/export-public-key`). Bundles carry no
tokens or secrets. See
[Integration reference](Integration%20Reference.md#workspace-portability-signed-export).

### Event-log integrity

Every event is hash-chained, and `GET /workspaces/:wid/events/verify` answers
`409` on a break. `MAIDAN_CHAIN_VERIFY_SECS=86400` checks every workspace daily
and counts `maidan_chain_verify_total{outcome}`. Alert on `broken` (tampering
or corruption) apart from `error` (the check could not run). Recovery is a
rebuild from the domain tables, not a repair of the log. See
[Integration reference](Integration%20Reference.md#event-log-hash-chain).

## Observability

### Metrics, dashboards and alerts

`GET /metrics` serves Prometheus text with no per-workspace labels. There is an
example [Grafana dashboard](https://github.com/david-engelmann/maidan/blob/main/docs/dashboards/maidan-operator.json)
and a set of [alert rules](https://github.com/david-engelmann/maidan/blob/main/docs/alerts/README.md),
tested with `scripts/check-alert-rules.sh`. Without Prometheus,
`MAIDAN_OPS_ALERT_CHANNEL` posts to Slack when a background loop panics,
stalls or keeps failing.

| Metric | Watch for |
|---|---|
| `maidan_http_shed_total`, `maidan_http_in_flight_requests` | Load shedding |
| `maidan_ws_connections`, `maidan_mcp_streamable_sessions` | Live connections |
| `maidan_indexer_events_behind`, `maidan_indexer_pending_age_seconds` | Search indexing lag |
| `maidan_outbox_pending`, `maidan_outbox_quarantined` | Event publishing stuck |
| `maidan_bus_listener_ok`, `maidan_bus_lag_total` | The `LISTEN` bus down; slow subscribers |
| `maidan_claims_reaped_total`, `maidan_claims_stalled_total` | Agents dying or stalling |

### Traces and logs

`OTLP_ENDPOINT` sends traces to an OpenTelemetry collector, and
`OTLP_METRICS=1` pushes metrics too; credentials in span headers print as
`Sensitive`. `MAIDAN_LOG_FORMAT=json` logs one JSON object per line. Each MCP
request logs one `info` line on `maidan_mcp::request` (method, tool, argument
names but never values, member, latency, outcome); add
`maidan_mcp::request=warn` to `MAIDAN_LOG` to drop it.

### Live updates

Subscribers use `GET /ws/subscribe` or `GET /mcp/stream` (SSE); the client
side is in [Integration](Integration.md). An SSE stream rechecks its token
every 10 s and ends with `event: stream_ended` once it is revoked. Both carry
the same control frames:

| Frame | When |
|---|---|
| `subscribe_ack` | After subscribing; carries `resume_token` and `after_id` for the next reconnect. |
| `replay_hint` | The subscriber lagged without a workspace filter, or replay failed. |
| `replay_truncated` | Replay returned 500 rows; resubscribe from the new `after_id` until it stops. |

By default delivery is best-effort: an event published out of order can be
skipped, and a slow subscriber can lag. A subscription with
`"at_least_once": true`, a workspace filter and a durable `consumer_id` (or
`?consumer_id=…&at_least_once=true` on `/mcp/stream`) gets every committed
event in `log_id` order, at a latency floor of `MAIDAN_DELIVERY_STABILITY_SECS`
(2 s) on fresh events. Keep that above your longest write transaction, and
have clients deduplicate by `log_id`.

### Delivery reliability metrics

| Metric | Means | Do |
|---|---|---|
| `maidan_bus_lag_total` rising | Subscribers behind the broadcast buffer | Make clients filter by workspace, so they replay on their own |
| `maidan_subscribe_replay_total{outcome="replay_hint"}` | Lag without a workspace filter | Fix the client filter |
| `maidan_subscribe_replay_total{outcome="replay_truncated"}` | Replay hit its 500-row page | The client loops on `after_id` |
| `maidan_bus_notify_hydrate_total{result="failed"}` | A `NOTIFY` named an unreadable event | Check database errors |
| `maidan_bus_listener_ok == 0`, `maidan_outbox_pending` high | Events not reaching other replicas | See below |
| `maidan_outbox_quarantined > 0` | Events stopped retrying | Fix the cause, then replay |
| `rebuild_needed` in logs | The search index has a gap | `maidan reindex-embeddings` |

Each write puts its event in `maidan_outbox` in the same transaction, and a
relay publishes it after commit, through `NOTIFY` on Postgres. When the
`LISTEN` connection is down: check the relay is running
(`maidan_outbox_relay_total` rising), and connect directly rather than through
a pooler that pins connections. Do not turn the relay off in production. On a
single process, `MAIDAN_OUTBOX_RELAY_MODE=polled` bridges the gap. Clients
recover from the event log with `after_id` or `resume_token`. Replay a
quarantined row with `POST /workspaces/{wid}/outbox/{id}/replay`.

### Search and reindexing

Lexical search works out of the box. Semantic search needs an embedding
provider: `hash-v1` works offline at low quality, or set
`MAIDAN_EMBEDDING_PROVIDER=openai-compatible` and point it at an embeddings
endpoint (see [Embeddings](Embeddings.md)). Postgres uses an HNSW index;
SQLite ranks in process. After changing the model, reindex:
`POST /operator/reindex-embeddings` starts a job and
`GET /operator/reindex-embeddings/:job_id` reports it (a `workspace_id` body
limits it to one workspace; without one it needs `operator:global`), or run
`maidan reindex-embeddings --database-url …`. Set `INDEXER_STALE_SECS=300` on
Postgres so readiness reports a stopped indexer.

## Upgrades

1. **Pick a release** and read the [CHANGELOG](../CHANGELOG.md) since yours.
   Pin that tag (or digest) for the server, CLI and Postgres images.
2. **Back up** the database and artifacts.
3. **Roll the server.** It migrates on boot, and migrations only add during
   the overlap ([Migrations](Migrations.md)). A release whose notes call for a
   cutover needs the old binary stopped first.
4. **Check** `/health/ready` and `GET /operator/status` before sending traffic.

Never run `maidan init` on an existing database; it refuses.

## API discovery and stability

`GET /openapi.json` describes every HTTP route and error (`info.version` is
the server release), and `GET /.well-known/agent-card.json` is the A2A Agent
Card. Erasure is `DELETE /messages/:id/purge` for a withdrawn message and
`POST /workspaces/:id/purge` for a whole workspace (both `token:admin`).
`GET /workspaces/:id/audit` is the workspace's audit trail.

Until the 1.0 launch, HTTP and MCP shapes can change between releases without
compatibility shims; the CHANGELOG says when they do. From 1.0 they are
stable. Release numbers count releases, so `v439.0.0` is a release number, not
a promise of stability.
