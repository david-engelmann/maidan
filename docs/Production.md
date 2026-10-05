# Production deployment

Guidance for running Maidan at `v1.0.0` and later. Security overview:
[Threat-Model.md](Threat-Model.md).

## Probes

| Endpoint          | Use        | Behavior                                      |
|-------------------|------------|-----------------------------------------------|
| `GET /health/live`  | Liveness   | Always `200` if the process is up.            |
| `GET /health/ready` | Readiness  | `200` when DB, artifact store, indexer (if stale check enabled and no embedding errors), and Postgres `LISTEN` bus (when used) are healthy. |
| `GET /health`       | Readiness  | Alias of `/health/ready`.                     |

### Operator status

`GET /operator/status` (`operator:global`) is the page to read before a deploy
or during an incident. It reports:

- **Phase:** `serving`, `degraded` (a readiness check fails) or `draining`.
- **Readiness checks:** the same ones `/health/ready` runs.
- **Search backfill:** the search tap's cursor against the event-log head, as a
  percentage and a count behind. Below 100% after a rebuild means the backfill
  is still running.
- **Replica lag,** in bytes, when a read replica is configured.
- **Queue depths:** the indexer queue, WebSocket connections against their
  ceiling, and the outbox.

It returns JSON, or a script-free page when the request sends
`Accept: text/html`.

### Rolling restarts

Kubernetes takes a terminating pod out of its Service at the same moment it
sends SIGTERM, and the endpoint change takes a few seconds to reach every
proxy. On SIGTERM the server therefore reports `/health/ready` as `503
{"status":"draining"}`, keeps serving for `MAIDAN_SHUTDOWN_DRAIN_SECS`, and only
then closes its listener and drains in-flight requests. Liveness stays `200`
throughout. The Helm chart and `k8s/base` set the drain to `10` and
`terminationGracePeriodSeconds` to `45`; the default outside them is `0`. The
runtime image is distroless, so this is done in-process rather than with an exec
`preStop` hook. A long-poll MCP call (up to 330 s) still open at the grace
period is cut, and clients retry.

## Environment

| Variable        | Required | Notes                                                |
|-----------------|----------|------------------------------------------------------|
| `DATABASE_URL`  | yes      | Postgres (recommended) or SQLite.                    |
|                 |          | SQLite connections enable `foreign_keys`, WAL, and `busy_timeout=5000` ms automatically. |
| `<NAME>_FILE`   | no       | A file holding the value of `<NAME>`, for `DATABASE_URL`, `MAIDAN_CONTENT_KEK`, `MAIDAN_SESSION_SECRET`, `MAIDAN_GITHUB_TOKEN`, `MAIDAN_GITHUB_WEBHOOK_SECRET`, `MAIDAN_SLACK_BOT_TOKEN` and `MAIDAN_SLACK_SIGNING_SECRET` (a Docker or Kubernetes secret mount), so the container's config holds a path instead of the secret. Read once at boot by the server and by `maidan`, with trailing newlines trimmed. Setting both `<NAME>` and `<NAME>_FILE` refuses boot, as does a file that cannot be read, is empty, or is not UTF-8; the error names the variable and the path, never the value. A `<NAME>` set to the empty string counts as unset. |
| `MAIDAN_ENV`    | no       | Set to `production` to forbid `AUTH_DISABLED` outright.       |
| `AUTH_DISABLED` | no       | Serve every request unauthenticated. **Fail-closed:** takes effect only when `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` is *also* set, and never when `MAIDAN_ENV=production` (either violation refuses boot). A stray `AUTH_DISABLED=1` alone now fails startup loudly instead of silently serving an open workspace. Dev/test/CI only. |
| `MAIDAN_ALLOW_INSECURE_NO_AUTH` | no | Explicit acknowledgement required to honor `AUTH_DISABLED`. Never set in production. |
| `MAIDAN_BOOTSTRAP` | no    | Set to `1` only during initial seed when auth is on **and** the server was built with the `bootstrap` Cargo feature (default for local dev; **off** in the production Docker image unless `MAIDAN_ENABLE_BOOTSTRAP=1` at image build). Allows unauthenticated `POST /workspaces` and `POST /workspaces/:wid/members`. Only the **first** workspace may be created via bootstrap; remove the flag and restart after minting tokens. |
| `FEDERATION_ENCRYPTION_KEY` | when federation is used | 32-byte secret (base64 or hex) used to encrypt peer outbound bearer tokens at rest. Required to create peers and for the poll worker after restart. Back up with your DB; rotation requires re-creating peers. |
| `MAIDAN_EXPORT_SIGNING_KEY` | to *produce* a signed workspace export | 32-byte Ed25519 seed (64-char hex or standard base64). `GET /workspaces/:id/export` and MCP `export_workspace` refuse until set — never an unsigned bundle. Back up with your other operator secrets; losing it does not strand existing files (the public key is in the artifact). |
| `MAIDAN_EXPORT_VERIFY_KEYS` | no | Comma-separated 32-byte public keys (hex or base64). When set, verify/import accept only those keys (authenticity pin). Empty / unset = integrity against the embedded key only — the blank-instance default. |
| `MAIDAN_CONTENT_KEK` | yes | 32-byte key-encryption key (64-char hex or standard base64) that wraps the per-message content keys (see *Crypto-shredding*). The server and `maidan init` refuse to start without it. Generate one with `openssl rand -hex 32`, keep it in your secret manager, never in data backups. |
| `MAIDAN_ALLOW_UNKNOWN_ENV` | no | `1` starts the server despite `MAIDAN_*` variables it does not know, logging them. Without it, boot refuses an unknown name and suggests the one it probably meant, so a misspelt variable cannot silently leave its default in place. In a Kubernetes pod the service-link variables Kubernetes injects for a Service named `maidan-…` (`MAIDAN_…_SERVICE_HOST`, `MAIDAN_…_PORT_8080_TCP`, …) are tolerated; the Helm chart and `k8s/` turn them off with `enableServiceLinks: false`. |
| `MAIDAN_ALLOW_INSECURE_DEV_KEK` | no | `1` lets a server with no `MAIDAN_CONTENT_KEK` use the built-in development key, which is public, with a warning. For local development and CI only; refused with `MAIDAN_ENV=production`. |
| `MAIDAN_CONTENT_KEK_PREVIOUS` | during a rotation | Comma-separated retired KEKs still able to unwrap. Requires `MAIDAN_CONTENT_KEK`. |
| `FEDERATION_DISABLED` | no | Set to `1` to disable the outbound poll worker. |
| `FEDERATION_POLL_INTERVAL_SECS` | no | Outbound poll interval (default `30`). |
| `MAIDAN_EMBEDDING_PROVIDER` | no | `hash-v1` (default) or `openai-compatible`. |
| `MAIDAN_EMBEDDING_ENDPOINT` | when provider is `openai-compatible` | Full URL to embeddings endpoint (OpenAI-compatible response shape). |
| `MAIDAN_EMBEDDING_MODEL` | when provider is `openai-compatible` | Embedding model id sent in request body. |
| `MAIDAN_EMBEDDING_API_KEY` | optional | Bearer token for remote provider. |
| `MAIDAN_EMBEDDING_DIM` | no | Embedding dimension for `openai-compatible`. Unset → the server embeds one probe string at boot and uses its length; set it to skip the probe. |
| `MAIDAN_EMBEDDING_TIMEOUT_SECS` | no | HTTP timeout for remote embeddings (default `15`). |
| `INDEXER_STALE_SECS` | no | When **> 0**, `/health/ready` is degraded when the embedding indexer is behind: the log holds a message event it has not handled, that event is older than this many seconds, and the indexer has made no progress for as long. An idle instance with nothing to index stays ready however long it has been quiet. An indexer error degrades readiness regardless. Default `0` (lag check disabled). **Recommended `300`** on Postgres deployments with embeddings enabled. "Handled" means handed to the embedding queue, so a backed-up provider shows in `maidan_indexer_queue_depth` and the indexer error, not here. |
| `OTLP_ENDPOINT` | no | gRPC OTLP collector URL for **traces** (and metrics when `OTLP_METRICS=1`). Each request's `debug` span carries its method, path (never the query, where an OAuth `code` travels) and headers, with `Authorization`, `Proxy-Authorization`, `Cookie`, `Mcp-Session-Id` and the Slack and GitHub signatures printed as `Sensitive`; so are `Set-Cookie` and `Mcp-Session-Id` on the response. |
| `OTLP_SERVICE_NAME` | no | Resource `service.name` for OTLP (default `maidan-server`). |
| `OTLP_METRICS` | no | Set to `1` to push the same `metrics` crate instruments to OTLP (fanout with Prometheus scrape). Requires `OTLP_ENDPOINT` unless `OTLP_METRICS_ENDPOINT` is set. |
| `OTLP_METRICS_ENDPOINT` | no | Override OTLP gRPC URL for metrics only. |
| `OTLP_METRICS_INTERVAL_SECS` | no | Periodic push interval (default `15`). |
| `MAIDAN_RATE_LIMIT_MAX` | no | Global HTTP rate limit per verified bearer. With no bearer, or a bearer that does not resolve, the client is the socket peer, or the `X-Forwarded-For` client when `MAIDAN_TRUSTED_PROXY_HOPS` > 0 — invented bearers do not each get a bucket. **Default `1200` per 60 s window**; set `0` to turn it off. `/health/*` and `/metrics` exempt. |
| `MAIDAN_VAPID_PRIVATE_KEY` / `MAIDAN_VAPID_PUBLIC_KEY` / `MAIDAN_VAPID_SUBJECT` | no | Web Push. All three enable a VAPID sender: base64url P-256 private scalar + uncompressed public key + a `mailto:` or `https:` contact. The board registers a push subscription; the router delivers when the member has no live WebSocket. A failed send is queued and retried. Unset → no web push (`vapid_unset`), and no key is generated. |
| `MAIDAN_WEBPUSH_LIVE_WINDOW_SECS` | no | Presence window (default `60`) for the "notify iff no live WS" gate: a member seen within this many seconds is treated as connected and not pushed. |
| `MAIDAN_WEBPUSH_WORKER_TICK_SECS` | no | How often the web push retry worker drains due sends (default `5`). |
| `MAIDAN_RATE_LIMIT_WINDOW_SECS` | no | Fixed window length in seconds (default `60`). Read only with an explicit `MAIDAN_RATE_LIMIT_MAX`; the built-in default is always 1200 per 60 s. |
| `MAIDAN_TRUSTED_PROXY_HOPS` | no | Number of rightmost reverse-proxy hops trusted when deriving the client IP from `X-Forwarded-For` (default `0`, so the header is ignored and the socket peer is used). Set this to the exact proxy/LB chain length; malformed or shorter chains fail closed to the socket peer. Bearer-token keys are unchanged. |
| `MAIDAN_ALLOW_PRIVATE_EGRESS` | no | Development-only escape hatch for loopback webhook/hook test receivers. Rejected when `MAIDAN_ENV=production`. Production operator-supplied HTTP targets are parsed canonically at registration; every delivery resolves again, rejects the whole answer set if any address is private/link-local/loopback/reserved, DNS-pins that set for the request, and never follows redirects. |
| `MAIDAN_RATE_LIMIT_REDIS_URL` | no | When set, global and per-token quotas use Redis fixed-window counters (multi-replica). Falls back to in-memory if unset or connection fails. |
| `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` | no | Per-workspace fairness limit: caps total requests for one workspace across **all** its tokens, on `/workspaces/{wid}/…` routes (incl. search), and only for a request authenticated into that workspace. A token for another workspace, or none, does not spend it. **Default `6000` per 60 s window**; set `0` to turn it off. Independent of the global limit; reuses the Redis backend when set. |
| `MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS` | no | Per-workspace fixed window in seconds (default `60`). Read only with an explicit `MAIDAN_WORKSPACE_RATE_LIMIT_MAX`; the built-in default is always 6000 per 60 s. |
| `MAIDAN_PRESENCE_HEARTBEAT_SECS` | no | Interval at which each replica re-announces its locally-connected members over `maidan_presence` (default `10`). Cross-replica presence is active only in Postgres NOTIFY mode. |
| `MAIDAN_PRESENCE_TTL_SECS` | no | A remote member with no heartbeat for this long is dropped from the merged roster (default `30`). Keep it a small multiple of the heartbeat. |
| `MAIDAN_DB_MAX_CONNECTIONS` | no | Pool size per process. Default **Postgres 16**, **SQLite 1** (SQLite serializes through one connection: concurrent read-modify-write transactions deadlock on the writer upgrade, which `busy_timeout` cannot resolve). See the replica caveat below. |
| `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS` | no | How long a request waits for a free pooled connection before erroring instead of hanging (default `30`). Under saturation this surfaces a clean `500`/timeout rather than blocking indefinitely. |
| `MAIDAN_DB_STATEMENT_TIMEOUT_MS` | no | Postgres per-connection `statement_timeout`. **Default `30000` (30 s)** — caps runaway queries so one can't pin a pooled connection indefinitely. Set `0` to disable. See the caveat below. |
| `MAIDAN_DB_IDLE_IN_TRANSACTION_TIMEOUT_MS` | no | Postgres `idle_in_transaction_session_timeout`. **Default `60000` (60 s).** A connection left idle inside an open transaction holds its locks and its snapshot, which blocks vacuum cluster-wide; this ends it. Set `0` to disable. |
| `MAIDAN_MAX_CONCURRENT_REQUESTS` | no | The most HTTP requests in flight at once. **Default `1024`; `0` turns it off.** Past it a request is refused at once with a `503` problem and `Retry-After: 1`, before the rate limiter or authentication run, so overload costs no database or Redis work. A request holds its slot until its response starts, so a long-poll MCP wait holds one while it waits; streaming bodies and WebSockets do not. `/health*` and `/metrics` are exempt. Watch `maidan_http_in_flight_requests` and `maidan_http_shed_total` (alert `MaidanLoadShedding`). |
| `MAIDAN_MAX_WS_CONNECTIONS` | no | The most concurrent `/ws/subscribe` connections accepted. **Default `10000`.** Past it an upgrade gets `503`, so long-lived sockets cannot exhaust the process; watch `maidan_ws_connections`. Client frames are capped at 64 KiB. |
| `MAIDAN_DB_LOCK_TIMEOUT_MS` | no | Postgres `lock_timeout`. **Default `10000` (10 s)**, so a request queued behind a held lock fails fast instead of piling up until `statement_timeout`. Boot migrations exempt themselves. Set `0` to disable. |
| `MAIDAN_DB_BUSY_TIMEOUT_MS` | no | SQLite `busy_timeout` (default `5000`). |
| `MAIDAN_CLAIM_REAP_TICK_SECS` | no | Seconds between claim-reaper ticks. **Default `5`; `0` turns it off.** Each tick frees every claim whose lease lapsed on an open thread (at most 1000 a tick) and emits `ClaimExpired` for the holder, so a dead agent's work returns to the queue and its supervisor hears about it on an idle channel too. The claim's worked time is charged to the thread's `max_wall_secs` budget in the same transaction; a claim that leaves the thread over budget gets `ClaimFailed` and a DLQ entry instead. Every replica runs it; Postgres `SKIP LOCKED` splits the work. Counter `maidan_claims_reaped_total`. |
| `MAIDAN_CLAIM_ACK_TIMEOUT_SECS` | no | How long a leased claim may stay unacknowledged before the claim reaper emits `ClaimUnacknowledged` for it, once per claim, and the owner and the holder's followers are notified of stuck work. **Default `120`; `0` turns it off.** The claim is not touched; its lease decides when it comes back. Runs on the reaper's tick, so it is off when `MAIDAN_CLAIM_REAP_TICK_SECS=0`. Counter `maidan_claims_unacknowledged_total`. |
| `MAIDAN_CLAIM_DEFAULT_LEASE_SECS` | no | The lease a `claim_next_thread` claim gets when the caller names none. **Default `600`.** Must be 1 s to 7 days, the bounds a requested lease or renewal is held to; an invalid value keeps the default and is logged. |
| `MAIDAN_DELIVERY_STABILITY_SECS` | no | At-least-once delivery (`v125.0.0`) stability window: a subscribe with `at_least_once` only delivers events whose insert time is older than this. Must exceed the longest insert-transaction duration. Default `2`; `0` disables the gate. |
| `MAIDAN_DELIVERY_RECONCILE_MS` | no | Poll cadence for the at-least-once reconcile loop (a NOTIFY also wakes it). Default `1000`. |

`GET /metrics` serves the Prometheus text exposition (HTTP, subscribe recovery,
indexer and bus gauges) with fixed label cardinality — no workspace UUIDs.

**Compiling OpenTelemetry out.** OTLP export is a default-on
cargo feature (`otel`) on `maidan-server`. Building with `--no-default-features`
(or a custom feature set that omits `otel`) drops the OpenTelemetry/OTLP/tonic
stack entirely for a leaner binary — plain `tracing` logs and the Prometheus
`/metrics` scrape are unaffected. In such a build the `OTLP_*` variables above are
inert (an `OTLP_ENDPOINT` that is set is reported to stderr at startup and
otherwise ignored). Leave the feature on (the default) to keep OTLP traces +
metrics push available.

**SCIM 2.0 provisioning.** An IdP (Okta / Entra ID / …) can provision and
deprovision workspace members via SCIM 2.0 at `/scim/v2/`:
`ServiceProviderConfig`, `Users` (create / read / list with `userName`,
`externalId` or `id eq` filters / replace / patch / delete) and `Groups` (create
/ read / list / replace / patch / delete). Point the IdP's SCIM connector at
`https://<host>/scim/v2` with a `token:admin` bearer token; everything it does
is confined to that token's workspace. No env config: the endpoint is always
available, gated on `token:admin`. Every change writes its audit row
(`scim.user.*`, `scim.group.*`, and one `token.revoke` per revoked token) in its
own transaction, so a change that cannot be recorded does not happen and the IdP
retries it.

- **Users.** A SCIM `id` is the Maidan member id and `userName` the member
  handle. Changing `userName`, by PUT (Okta) or by PATCH with
  `"path": "userName"` or a pathless value object (Entra ID), renames the
  member: the id stays, so tokens, messages, claims and group memberships stay
  with it, and every surface that shows the handle shows the new one. A
  `userName` another member of the workspace holds is `409` with
  `scimType: uniqueness`. Deactivation (`active=false`, also Entra ID's string
  `"False"`) and delete revoke the member's API tokens; delete also removes the
  user from every group. `displayName` is set at creation and not changed
  afterwards.
- **Groups.** A group is the IdP's named set of users it provisioned into the
  workspace. It records membership and grants nothing by itself: no channel
  access and no capability follow from it. Members must be SCIM users of the
  same workspace; any other id (another workspace's member, an agent, an
  unknown id) is `400` with `scimType: invalidValue`. PATCH accepts `add`,
  `remove` and `replace` of `members` in both the Okta form (`remove` with the
  filter path `members[value eq "<id>"]`) and the Entra ID form (path
  `members` with a value array, capitalized `op`), `remove` of `members` with
  no value empties the group, and `replace` of `displayName` and `externalId`
  with a path or a pathless value object. The list filters on `displayName eq`
  (not case-sensitive), `externalId eq` or `id eq`, pages with `startIndex` and
  `count` (at most 200), and honours `excludedAttributes=members`; any other
  filter is `400` with `scimType: invalidFilter`.
- **Filters.** `Users` filters on `userName eq` (not case-sensitive),
  `externalId eq` or `id eq`, and answers with that workspace's matching users
  only. Any other filter, on `Users` or `Groups`, is `400` with
  `scimType: invalidFilter`, never the whole list, so an IdP matching on an
  attribute Maidan does not filter cannot link to the wrong user.
- **Not supported:** `Bulk`, sorting, ETags, `Schemas` and `ResourceTypes`,
  nested groups, compound (`and`/`or`) filters, and paging on `Users`.

### Database tuning (`v107.0.0`)

- **Total connections = replicas × `MAIDAN_DB_MAX_CONNECTIONS`.** Behind a load
  balancer this must stay under Postgres `max_connections` (default 100) with
  headroom for migrations, the bus `LISTEN` connections, and admin tools. E.g.
  4 replicas × 16 = 64. Raise the pool only after confirming the server is
  connection-starved (acquire timeouts), not query-bound.
- **`MAIDAN_DB_STATEMENT_TIMEOUT_MS` applies to every server query**, including
  the in-server operator reindex (`POST /operator/reindex-embeddings`). The
  default is now `30000` (30 s); raise it above your longest expected query, or
  trigger large reindexes via the `maidan reindex-embeddings` CLI, which uses its
  own pool with no cap, or set `0` to disable the cap entirely. Boot migrations
  are already exempt (the migration session resets the timeout under the advisory
  lock), so the default will not break startup or a rolling update.

### Tenant fairness (`v110.0.0`)

On a shared instance, `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` bounds the total request
rate for any single workspace (across all its tokens) on `/workspaces/{wid}/…`
routes — so one tenant's heavy loop (a tight semantic-search poll, a backfill)
can't monopolize the connection pool and degrade search/write latency for
others. The budget is charged only once the caller is authenticated into that
workspace, so naming the id is not enough to spend it. It is on by default at 6000 requests per 60 s (about 100 a second):
five clients each at the per-client default of 1200 per 60 s, so a busy
workspace's agents meet their own limits before the shared one, and well under
what one node serves (666–1586 requests a second on the SQLite
[benchmark](Benchmark.md)). Set a value to change it, or `0` to turn it off.
It is **independent** of the per-client `MAIDAN_RATE_LIMIT_MAX`. With
`MAIDAN_RATE_LIMIT_REDIS_URL` set, the per-workspace counter is shared across
replicas; otherwise it is per-process, so N replicas without Redis admit up to N
times the limit. Not a substitute for hard CPU/IO isolation — that is
infra-level (separate instances / Postgres resource groups).

### Local embedding servers (e.g. LM Studio)

Maidan's indexer uses the **OpenAI-compatible embeddings** API shape, not chat
completion. Point `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` at your server's
**embeddings** URL (for example `http://localhost:1234/v1/embeddings`) and set
`MAIDAN_EMBEDDING_MODEL` to the loaded model id. A chat endpoint such as
`http://localhost:1235/api/v1/chat` is not used for search indexing.

## Bootstrap

### `maidan init` (recommended)

The `maidan` CLI seeds the first admin directly through the store, so a production
deployment needs no unauthenticated HTTP routes and no `AUTH_DISABLED`:

```sh
DATABASE_URL=postgres://… MAIDAN_CONTENT_KEK=… maidan init --workspace my-team --admin-handle david
```

For containerized deployments, the separately published CLI image runs the
same release binary without adding a shell or operator tooling to the
distroless server image. Put it on a network that can reach the database and
pin it to the server's exact tag:

```sh
MAIDAN_TAG=v412.0.0
MAIDAN_NETWORK=your_database_network
docker run --rm --network "$MAIDAN_NETWORK" \
  -e DATABASE_URL=postgres://maidan:…@postgres/maidan \
  -e MAIDAN_CONTENT_KEK \
  "ghcr.io/david-engelmann/maidan-cli:${MAIDAN_TAG}" \
  init --workspace my-team --admin-handle david
```

It runs migrations, creates the initial workspace and an admin member, mints an
all-capabilities bearer token, and prints that token **once** (to stdout; save it).
It **refuses if the database already has a workspace**, so it can never clobber an
existing deployment or mint a second root token. Use the printed token to mint
narrower per-agent tokens via the API. The production image can stay
bootstrap-stripped (`--no-default-features`), since `init` writes through the store
rather than the bootstrap HTTP routes. The CLI image is a one-shot operator
tool, not a long-running sidecar.

On a release tag, GitHub Release publication waits for a smoke that pulls the
published server, CLI, and Postgres images by that exact tag. It initializes a
fresh database through the CLI image, requires the server to report the tag as
healthy, authenticates `/me` with the one-time token, and confirms anonymous
access is rejected. A source checkout or locally built image cannot satisfy
that release gate.

### A second workspace

`maidan init` stops after the first workspace. A later tenant is
`POST /operator/workspaces` with the init token (it holds `operator:global`):

```sh
curl -sS -X POST "$MAIDAN_URL/operator/workspaces" \
  -H "Authorization: Bearer $MAIDAN_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"name":"other-team","admin_handle":"ada"}'
```

The response is the workspace, its admin member, and that admin bearer token,
shown once. The token can administer that workspace, including minting narrower
tokens. It does not hold `operator:global` or `audit:read-global`, and a later
mint cannot add those unless the caller already holds them. No
`MAIDAN_BOOTSTRAP`, and the route exists on the production image, which is
built without the bootstrap routes.

### HTTP bootstrap (development only)

`maidan init` above is how a deployment gets its first token, on a private network
too. The published server image is built without the `bootstrap` Cargo feature, so
the routes below do not exist in it.

A development build with the feature (`cargo run`, or
`docker build --build-arg MAIDAN_ENABLE_BOOTSTRAP=1 -f crates/maidan-server/Dockerfile .`)
serves unauthenticated `POST /workspaces` and `POST /workspaces/:wid/members` when
`MAIDAN_BOOTSTRAP=1` is set. Only the **first** workspace may be created that way; a
second `POST /workspaces` returns `403`. Those routes mint no token, so a development
seed over HTTP also runs without auth, which needs both variables:

1. `AUTH_DISABLED=1` and `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` (the acknowledgement:
   `AUTH_DISABLED` alone refuses to boot, and with `MAIDAN_ENV=production` it refuses
   regardless), plus `MAIDAN_BOOTSTRAP=1`.
2. Create the workspace and member, mint an admin token.
3. Unset all three and restart with auth on.

Integration tests use `AUTH_DISABLED=1` + `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` (bootstrap flag not required).

Human browser login via OIDC ships in **`v2.0.0`**. See [OIDC.md](OIDC.md) for design
detail. Summary:

| Variable | Required | Notes |
|----------|----------|-------|
| `MAIDAN_OIDC_ENABLED` | when using OIDC | `1` enables `/auth/oidc/*` and session routes. |
| `MAIDAN_SESSION_SECRET` | when OIDC is on; optional otherwise | HMAC key for signed `maidan_session` cookies and subscribe `resume_token`s (32+ bytes). Required at startup when OIDC is on; bare session UUIDs in cookies are rejected. Without OIDC it signs the sessions `/ui/` makes from a pasted token (`POST /auth/session/from-token`); unset, there are none and the page keeps a pasted token in the tab only. |
| `MAIDAN_OIDC_ISSUER` | yes (non-mock) | IdP issuer URL for discovery. |
| `MAIDAN_OIDC_CLIENT_ID` | yes (non-mock) | OAuth client id. |
| `MAIDAN_OIDC_CLIENT_SECRET` | confidential clients | Code exchange secret. |
| `MAIDAN_OIDC_REDIRECT_URI` | yes | Registered callback (e.g. `https://host/auth/oidc/callback`). |
| `MAIDAN_OIDC_MOCK` | no | `1` for deterministic dev/CI only; forbidden when `MAIDAN_ENV=production`. |
| `MAIDAN_OIDC_FIRST_ADMIN` | no | Default on: session may mint the first `token:admin` per workspace via `POST /auth/session/mint`. Set `0` to disable. |
| `MAIDAN_COOKIE_SECURE` | no | Set `1` in production for `Secure` session cookies. |
| `MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI` | no | Registered post-logout redirect (e.g. `https://host/ui/`). Used when IdP exposes `end_session_endpoint`. |
| `MAIDAN_OIDC_AUTO_MINT` | no | `1` redirects to `/ui/?auto_mint=1` after login when the workspace has no `token:admin` yet; the UI then calls `POST /auth/session/mint`. Off by default. Requires first-admin mint (`MAIDAN_OIDC_FIRST_ADMIN` not `0`). |
| `MAIDAN_SESSION_TTL_SECS` | no | Browser session lifetime (default `28800`, 8 hours), OIDC or token. A token's session also ends with the token. |
| `MAIDAN_SUBSCRIBE_RESUME_SECRET` | no | Override HMAC key for subscribe resume tokens only. |
| `MAIDAN_SUBSCRIBE_RESUME_TTL_SECS` | no | Resume token lifetime in seconds (default `3600`). |

### Experimental Jev land-gate advisor

The decision-model spike is **off by default** and does not participate in the
authoritative land-gate write or close paths. Enabling it adds only
`POST /threads/:id/land-gate/advice`; see
[Agent integration — Experimental Jev advice](Integration.md#experimental-jev-advice-default-off)
for its request, response, calibration harness, and privacy boundary.

| Variable | Default | Notes |
|----------|---------|-------|
| `MAIDAN_JEV_LAND_GATE_ENABLED` | `0` | `1` enables the advisory route. Invalid values fail startup. |
| `TYPESAFE_API_KEY` | — | Required only when the feature is enabled. Never sent to clients or logged. |
| `MAIDAN_JEV_MODEL` | `jev-latest` | Model or alias returned by TypeSafe `GET /v1/models`. |
| `MAIDAN_JEV_GREEN_MIN_CONFIDENCE` | `0.90` | Green below this becomes advisory amber. Must be in `[0,1]`. |
| `MAIDAN_JEV_RED_MIN_CONFIDENCE` | `0.90` | Red below this becomes advisory amber. Must be in `[0,1]`. |
| `MAIDAN_JEV_TIMEOUT_MS` | `1500` | Positive per-request timeout. A timeout returns `502` only on the advice call. |
| `MAIDAN_JEV_BASE_URL` | `https://api.typesafe.ai/` | Origin only. Uses the shared DNS-pinned SSRF/redirect guard. |

When enabled, a missing key, malformed threshold, unsafe base URL, or DNS
resolution failure stops startup rather than silently enabling a partial
configuration. With the flag unset or `0`, none of the other variables are
read and the server behaves exactly as before.

After OIDC login, use `/ui/` (session cookie) or mint an API token for MCP.

**The board (`/ui/`):** channels and tasks, and the thread when one is open.
Posts go through `/ui/api/...` on the session cookie. A token is pasted on the
first-run card — those fields sit behind Change in the header once you are in —
and exchanged for a session with that token's authority
(`POST /auth/session/from-token`). The page does not keep the token. Revoking
or rotating it ends the session.

**Sessions.** A session row (`maidan_sessions`) names its member and workspace
and, for one made from a token, the token's id. It is checked on every request:
an expired row, or one whose token is no longer live, is deleted and refused.
Signing out (`POST /auth/logout`) deletes the row. CSRF is handled by
`SameSite=Lax`, JSON request bodies, and refusing an unsafe session request or a
session WebSocket from another origin; the session keeps no CSRF secret. Behind
a proxy that rewrites `Host`, browsers still send `Sec-Fetch-Site`, which the
check prefers. Creating a session writes an audit row in the same transaction
(`session.from_token`, or `session.create` for an OIDC login).
Remove `MAIDAN_BOOTSTRAP` once the first human has `token:admin`.

## One instance built from `main`

The quickstart and the release images run a tagged release. A shared instance
that tracks `main` is built from the repository at a pinned commit instead. The stack that runs it
lives outside this repository; this section is what it needs from here.

**1. Build the images at one commit.** Record the full commit SHA where the
stack is defined, and rebuild only by changing it. The server, the database
image and the CLI come from the same commit, because `maidan init` applies that
commit's migrations.

```sh
git clone https://github.com/david-engelmann/maidan.git && cd maidan
git checkout "$COMMIT"            # a full SHA on main
export MAIDAN_TAG="main-$(git rev-parse --short=12 HEAD)"

docker build -f crates/maidan-server/Dockerfile --build-arg MAIDAN_VERSION="$MAIDAN_TAG" \
  -t "maidan-server:$MAIDAN_TAG" .
docker build -f docker/Dockerfile.db -t "maidan-postgres:$MAIDAN_TAG" .   # pgvector

# The CLI image wraps a binary built for Debian bookworm, like CI's release CLI image.
docker run --rm -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/src/target-cli \
  rust:1.91-slim-bookworm sh -c \
  'apt-get update -qq && apt-get install -y -qq pkg-config libssl-dev >/dev/null \
   && cargo build --release -p maidan-cli'
mkdir -p cli-image && cp target-cli/release/maidan cli-image/maidan
cp docker/Dockerfile.cli cli-image/Dockerfile
docker build --build-arg MAIDAN_VERSION="$MAIDAN_TAG" -t "maidan-cli:$MAIDAN_TAG" cli-image
```

Put `MAIDAN_TAG` in the stack's `.env` so compose runs these images. The
server image is built without the `bootstrap` feature, so it has no
unauthenticated seed routes and `MAIDAN_BOOTSTRAP` does nothing.

**2. Choose the database.**

| | Postgres (`maidan-postgres`) | SQLite |
|---|---|---|
| Containers | one more | none |
| Writers | a pool of 16 connections; replicas can share it | one connection; every write waits its turn |
| Live events across processes | `LISTEN`/`NOTIFY`, so a second replica works | in memory, one process only |
| Semantic search | pgvector with an HNSW index | cosine over stored vectors, no index |
| Backup | `pg_dump`, point-in-time recovery | `scripts/backup.sh` (`VACUUM INTO`, needs the `sqlite3` CLI beside the file) |

Use Postgres for an instance several agents write to at once. SQLite suits one
operator and a few agents, and is one file to copy away (with the server
stopped, or through `scripts/backup.sh`).

**3. Write the secrets to files.** Each secret-bearing variable is read from
the file `<NAME>_FILE` names when it is set (see [Environment](#environment)),
so the container's config holds paths and `docker inspect` shows no secret. The
server runs as uid `65532`; give it the files:

```sh
mkdir -p secrets
openssl rand -hex 32 > secrets/content_kek          # back this up apart from the data
openssl rand -hex 32 > secrets/session_secret
openssl rand -hex 24 > secrets/postgres_password
echo "postgres://maidan:$(cat secrets/postgres_password)@maidan-postgres:5432/maidan" \
  > secrets/database_url
# secrets/github_token, github_webhook_secret, slack_bot_token, slack_signing_secret:
# paste each value, one per file.
sudo chown 65532:65532 secrets/* && sudo chmod 600 secrets/*
```

`MAIDAN_GITHUB_TOKEN` is a **personal access token** with exactly two
repository permissions, `contents:write` and `pull_requests:write`
(fine-grained: Contents and Pull requests, read and write), on exactly the
repositories the change flow may write to; [Result delivery to GitHub and
Slack](#result-delivery-to-github-and-slack) says why nothing else. Postgres's
own password goes through the Postgres image's `POSTGRES_PASSWORD_FILE`.

**4. The services.** Auth is on: nothing sets `AUTH_DISABLED`, and
`MAIDAN_ENV=production` refuses it and the development KEK outright. With
`production` the session cookie is `Secure`, so reach the board over HTTPS or
on `localhost`; bearer tokens work either way.

```yaml
services:
  maidan-postgres:
    image: maidan-postgres:${MAIDAN_TAG}
    environment:
      POSTGRES_USER: maidan
      POSTGRES_DB: maidan
      POSTGRES_PASSWORD_FILE: /run/secrets/postgres_password
    secrets: [postgres_password]
    volumes: [maidan_pg:/var/lib/postgresql/data]
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U maidan -d maidan"]
      interval: 2s
      retries: 30

  # The image has no /data, so a new named volume belongs to root; hand it to
  # the server's uid before anything writes artifacts (or SQLite) there.
  maidan-volume-init:
    image: busybox:1.36
    command: ["chown", "65532:65532", "/data"]
    volumes: [maidan_data:/data]

  maidan:
    image: maidan-server:${MAIDAN_TAG}
    depends_on:
      maidan-postgres: { condition: service_healthy }
      maidan-volume-init: { condition: service_completed_successfully }
    environment:
      MAIDAN_ENV: production
      MAIDAN_BIND: 0.0.0.0:8080
      ARTIFACT_BACKEND: localfs
      ARTIFACT_LOCALFS_ROOT: /data/artifacts
      DATABASE_URL_FILE: /run/secrets/database_url
      MAIDAN_CONTENT_KEK_FILE: /run/secrets/content_kek
      MAIDAN_SESSION_SECRET_FILE: /run/secrets/session_secret
      MAIDAN_GITHUB_TOKEN_FILE: /run/secrets/github_token
      MAIDAN_GITHUB_WEBHOOK_SECRET_FILE: /run/secrets/github_webhook_secret
      MAIDAN_SLACK_BOT_TOKEN_FILE: /run/secrets/slack_bot_token
      MAIDAN_SLACK_SIGNING_SECRET_FILE: /run/secrets/slack_signing_secret
    secrets:
      - database_url
      - content_kek
      - session_secret
      - github_token
      - github_webhook_secret
      - slack_bot_token
      - slack_signing_secret
    volumes: [maidan_data:/data]
    healthcheck:
      test: ["CMD", "/usr/local/bin/maidan-server", "--health-check"]
      interval: 5s
      retries: 30
      start_period: 10s

  # One-shot, run once by hand: `docker compose run --rm maidan-init`.
  maidan-init:
    image: maidan-cli:${MAIDAN_TAG}
    profiles: [init]
    depends_on:
      maidan-postgres: { condition: service_healthy }
      maidan-volume-init: { condition: service_completed_successfully }
    command: ["init", "--workspace", "my-team", "--admin-handle", "ops"]
    environment:
      DATABASE_URL_FILE: /run/secrets/database_url
      MAIDAN_CONTENT_KEK_FILE: /run/secrets/content_kek
    secrets: [database_url, content_kek]
    volumes: [maidan_data:/data]

secrets:
  postgres_password: { file: ./secrets/postgres_password }
  database_url: { file: ./secrets/database_url }
  content_kek: { file: ./secrets/content_kek }
  session_secret: { file: ./secrets/session_secret }
  github_token: { file: ./secrets/github_token }
  github_webhook_secret: { file: ./secrets/github_webhook_secret }
  slack_bot_token: { file: ./secrets/slack_bot_token }
  slack_signing_secret: { file: ./secrets/slack_signing_secret }

volumes:
  maidan_pg:
  maidan_data:
```

For SQLite, drop `maidan-postgres`, its volume and password, and the
`depends_on` entries that name it, and in both `maidan` and `maidan-init`
replace `DATABASE_URL_FILE` and its secret with
`DATABASE_URL: sqlite:///data/maidan.db?mode=rwc` (a path is not a secret).
The two then share the database file through `maidan_data`.

A variable set both ways (`MAIDAN_GITHUB_TOKEN` and `MAIDAN_GITHUB_TOKEN_FILE`)
or a file that cannot be read refuses boot and names the variable. The server
logs which variables came from files, never their values.

**5. The first workspace and token.** Run init once, then start the server:

```sh
docker compose run --rm maidan-init
docker compose up -d --wait maidan
```

Init migrates the database, creates the workspace and its admin member, and
prints the workspace id and an admin bearer token holding every
capability, once. Keep the token in the stack's secret store; it is the
`token:admin` the next step needs, and the one to mint narrower agent tokens
from. A second run refuses. No `MAIDAN_BOOTSTRAP` and no unauthenticated route
is involved.

**6. Seed the egress allowlist.** Nothing is allowed by default. Each row is
one audited call, `POST /workspaces/{wid}/egress-targets` with
`{surface, selector}` and a `token:admin` bearer, and writes an
`egress_target.allow` audit row. A `github_branch` row (`owner/name@base`)
lets the change flow commit to an agent branch of that repository and open a
draft pull request into that base, one row per base; a `github` row
(`owner/name`) allows result comments only; a `slack` row is a channel id. With
example repositories:

```bash
MAIDAN=http://maidan:8080; WID=<workspace id from init>; ADMIN_TOKEN=<token from init>
URL="$MAIDAN/workspaces/$WID/egress-targets"
H1="Authorization: Bearer $ADMIN_TOKEN"; H2='Content-Type: application/json'

curl -fsS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github_branch","selector":"example/app@dev"}'
curl -fsS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github_branch","selector":"example/skills@main"}'
curl -fsS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github","selector":"example/app"}'
curl -fsS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github","selector":"example/skills"}'
curl -fsS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"slack","selector":"C0123456789"}'
```

`GET "$URL"` lists the rows; `DELETE "$URL/<id>"` revokes one.

**7. Smoke check.** From any container on the stack's network (the server image
has no shell or `curl`; its own probe is `maidan-server --health-check`):

```bash
curl -fsS "$MAIDAN/health/ready"                                   # 200: database, artifacts, bus
test "$(curl -s -o /dev/null -w '%{http_code}' "$MAIDAN/me")" = 401  # auth is on
curl -fsS "$MAIDAN/me" -H "$H1" | jq -e --arg w "$WID" '.workspace_id == $w'
curl -fsS "$URL" -H "$H1" | jq -e 'length >= 5'                    # the seed is in place
```

And on the Docker host, that the container's config carries paths and no
secret (with SQLite, take `DATABASE_URL|` out of the pattern):

```bash
docker inspect "$(docker compose ps -q maidan)" --format '{{json .Config.Env}}' | jq -e \
  'map(select(test("^(DATABASE_URL|MAIDAN_(CONTENT_KEK|SESSION_SECRET|GITHUB_TOKEN|GITHUB_WEBHOOK_SECRET|SLACK_BOT_TOKEN|SLACK_SIGNING_SECRET))="))) | length == 0'
```

**Upgrading.** Change `COMMIT`, rebuild the three images and restart the
server; it migrates forward on boot. Back up first (see [Backup & disaster
recovery](#backup--disaster-recovery-v26000)). Do not run init again.

## API discovery

| Endpoint            | Use                                      |
|---------------------|------------------------------------------|
| `GET /openapi.json` | Machine-readable OpenAPI 3.1 (Track W.1). HTTP routes and `application/problem+json` errors; subscribe/resume protocol summary in `info.description`. Auth/session routes are under the `auth` tag (`/auth/oidc/*`, `/auth/session`, `/ui/api/...`). |
| `GET /workspaces/:wid/search` | See [Search](#search-get-workspaceswidsearch). OpenAPI `SearchHit` documents `embedding_model`. |
| `GET /metrics`    | Prometheus text (HTTP counters, subscribe replay, indexer age, bus listener). |
| `DELETE /messages/:id/purge` | Hard-delete a **tombstoned** message (GDPR erasure); requires `token:admin`. |
| `POST /workspaces/:id/purge` | Deep workspace erasure (`v28.0.0`): tombstone+purge all messages, remove embeddings/references, revoke API tokens, delete event log; returns counts JSON. Requires `token:admin`. |
| `GET /workspaces/:id/audit` | Workspace-scoped audit trail (`workspace:read`): the rows stamped with that workspace. |

Import into Swagger UI, Redoc, or your client generator. The document
version tracks the server release (`info.version`).

## A2A transports (`v282.0.0`+)

Maidan speaks the [A2A protocol](https://a2a-protocol.org) across three bindings, all
over the same operations and auth:

| Binding | Endpoint | Default |
|---------|----------|---------|
| JSON-RPC | `POST /a2a/v1/rpc` | always on |
| HTTP+JSON/REST | `/a2a/v1/*` (e.g. `POST /a2a/v1/message:send`, `GET /a2a/v1/tasks/{id}`) | always on |
| gRPC | the official `lf.a2a.v1.A2AService` (every operation) on a separate port | **opt-in** |

The **Agent Card** (`GET /.well-known/agent-card.json`) advertises the available
interfaces so clients can negotiate a transport (A2A §5.2). Configure it for your
deployment:

| Env | Effect |
|-----|--------|
| `MAIDAN_A2A_GRPC_ADDR` | Bind address for the gRPC server (e.g. `0.0.0.0:50051`). Unset ⇒ gRPC off. An address that does not parse refuses boot. |
| `MAIDAN_A2A_GRPC_PLAINTEXT` | `1` acknowledges that TLS is terminated in front of the gRPC listener. Required when `MAIDAN_A2A_GRPC_ADDR` is not a loopback address; without it the server refuses to start. |
| `MAIDAN_A2A_PUBLIC_ORIGIN` | e.g. `https://maidan.example`. Makes the card's HTTP interface URLs absolute. Unset ⇒ host-relative. |
| `MAIDAN_A2A_GRPC_PUBLIC_ADDR` | The reachable gRPC `host:port` to advertise (distinct from the bind address, so it's correct behind a proxy/LB). Set this to add a `GRPC` interface to the card. |

The gRPC listener speaks plaintext HTTP/2 and does not terminate TLS, and calls
carry the bearer token in their metadata, so the server refuses to bind it off
loopback until `MAIDAN_A2A_GRPC_PLAINTEXT=1` says TLS is handled in front. Do
not expose it to an untrusted network: put it behind a TLS-terminating ingress or proxy, and keep the hop from
the proxy to Maidan on a trusted private network. Expose the gRPC port in your
deployment (Kubernetes Service / compose port) only that way; the HTTP bindings
share the main HTTP port.

A2A push configs seal their `token` and credentials with `FEDERATION_ENCRYPTION_KEY`;
without it, creating a push config that carries either fails. Push targets pass the
same egress checks as webhooks, and `secret://` references in a pushed task are
substituted like a webhook's (see *Secret substitution on egress*).

## WebSocket and MCP subscribe (`v4.0.0`)

Real-time subscribers use **`GET /ws/subscribe`** (WebSocket) or **`GET /mcp/stream`**
(SSE). Both share the same control frames and event envelope shape.

MCP resource subscription notifications use **`GET /mcp/notifications`** (SSE JSON-RPC
lines) with **`POST /mcp`** for `resources/subscribe` / `tools/call` — requires
`workspace:read` (same as resource read). Distinct from `/mcp/stream` workspace events.
A listener receives only what its own credential subscribed to, for resources it can
still read. Behind a load balancer no affinity is needed: a stateless client's
subscriptions are kept in the database (`maidan_mcp_resource_subscriptions`), and the
replica holding its listener delivers them over the Postgres NOTIFY path. They lapse
once no replica has had a listener open for the caller for
`MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS` (default 3600). A `2024-11-05` session's
subscriptions stay in the replica holding the session.

**Semantic search:** Postgres uses `pgvector`; SQLite uses stored 1024-dim embeddings
with cosine ranking (dev parity, no HNSW index on SQLite).

### First message (WebSocket)

Send one text frame after connect:

```json
{
  "filter": { "workspace_id": "<uuid>", "kinds": ["message_posted"] },
  "after_id": 0,
  "token": "<bearer when auth enabled>"
}
```

Or reconnect with only:

```json
{ "resume_token": "<from subscribe_ack>", "token": "<bearer>" }
```

Invalid or expired `resume_token` closes the socket with code **1008**.

### MCP SSE query

`GET /mcp/stream?workspace_id=<uuid>&after_id=0` or
`?resume_token=<opaque>`. Requires bearer with `event:subscribe`.

### Control frames

| `type`              | When | Fields |
|---------------------|------|--------|
| `subscribe_ack`     | After subscribe / replay | `resume_token`, `after_id` (watermark for next resume) |
| `replay_hint`       | Bus lag without workspace scope (or replay failure) | `skipped`, `after_id`, optional `workspace_id`, `replay` URL |
| `replay_truncated`  | Event-log replay returned 500 rows | `after_id` (new watermark), `limit` (`500`), optional `workspace_id` |

Loop: on `replay_truncated`, reconnect or resubscribe with `after_id` (or a fresh
`resume_token` from the next `subscribe_ack`) until no truncation frame.

Event envelopes follow: `{ "log_id": <i64>, "kind": "...", ... }`.

### At-least-once delivery (`v125.0.0`)

By default the live path is **optimistic, best-effort**: events stream with low
latency, but an event published out of `log_id` order (a failed outbox row
retried after later rows, or a late-committing serial) can be silently skipped
by the monotonic watermark, and the live buffer can drop events on lag.

Set **`at_least_once`** (requires both a workspace filter and a durable
`consumer_id`) to switch that subscription to **cursor-driven reconcile**
delivery — on **WebSocket** (`/ws/subscribe` frame) or **MCP SSE**
(`/mcp/stream` query param), `v126.0.0`:

```json
{ "filter": { "workspace_id": "<uuid>" }, "consumer_id": "my-agent", "at_least_once": true }
```

```
GET /mcp/stream?workspace_id=<uuid>&consumer_id=my-agent&at_least_once=true
```

- **Guarantee:** every committed event matching the filter is delivered in
  `log_id` order and exactly once per `consumer_id` — no silent gaps. The durable
  delivery cursor floors re-delivery across reconnects.
- **Cost:** a stability-window latency floor on *fresh* events
  (`MAIDAN_DELIVERY_STABILITY_SECS`, default `2s`); the backlog (already stable)
  is delivered immediately on connect.
- **Caveat:** strictness holds under "no insert transaction outlives the window".
  A pathologically long (`> window`) write transaction can still strand a lower
  `log_id`; size the window above your slowest write transaction. Clients should
  still dedup by `log_id` (cheap, and the contract is at-least-once).

### Delivery reliability metrics (`v6.0.0`)

Scrape `GET /metrics` and alert on subscribe recovery paths (labels are fixed —
no per-workspace series).

| Metric | Symptom | Suggested action |
|--------|---------|------------------|
| `maidan_bus_lag_total` rising | In-process subscribers falling behind the broadcast buffer | Check publish rate; scale consumers; ensure clients use `workspace_id` filter for auto-replay |
| `maidan_subscribe_replay_total{outcome="replay_hint"}` | Lag without workspace scope or auto-replay failed | Fix client filter; inspect store/DB errors in logs |
| `maidan_subscribe_replay_total{outcome="replay_truncated"}` sustained | Event log replay hitting 500-row window | Client should loop on `after_id` / `resume_token` until truncation stops |
| `maidan_indexer_pending_age_seconds` high, or `/health/ready` `indexer` reports "indexer is behind" (with `INDEXER_STALE_SECS` set) | Messages are queued in the event log and the indexer is not taking them | Check embedding provider errors on `/health`; verify indexer task running. A high `maidan_indexer_last_event_age_seconds` alone is an idle instance, not a fault |
| Indexer `rebuild_needed` / `RebuildRequired` in logs | Search tap hit a chain break or `Lagged` without a durable log | Do not keep serving the gapped index. Reindex from the messages table (`maidan reindex-embeddings`). A peer that missed a pruned prefix takes `GET /workspaces/:id/snapshot` then `…/events/catch-up` — never clamp |
| `maidan_bus_listener_ok == 0` | Postgres `LISTEN` task degraded | Inspect DB connectivity; `maidan_bus_listener_errors_total` trend |

### Postgres bus NOTIFY pointers (`v7.0.0`)

Production mutations append to `maidan_events` before `pg_notify`. The NOTIFY
payload is a small `log_id_v1` pointer; the server hydrates the row before
fan-out. Very large message bodies are limited by the database row, not the
legacy ~8KB NOTIFY cap.

Direct `bus.publish` without a prior `append_event` (tests only) still uses
full JSON on NOTIFY and can hit `PayloadTooLarge`. Operators should rely on HTTP
mutations or federation ingest for large events.

### Bus hydrate metrics (`v8.0.0`)

Postgres pointer delivery records hydrate outcomes on `/metrics`:

| Metric | Symptom | Suggested action |
|--------|---------|------------------|
| `maidan_bus_notify_hydrate_total{result="not_found"}` rising | NOTIFY referenced a `log_id` with no `maidan_events` row | Audit publish order (append before notify); check replication lag; verify no manual `pg_notify` with stale ids |
| `maidan_bus_notify_hydrate_total{result="failed"}` rising | Row present but payload corrupt or DB errors during hydrate | Inspect `maidan_events` payload JSON; check DB errors in logs |
| `maidan_bus_notify_hydrate_total{result="invalid_payload"}` | Malformed NOTIFY JSON (not pointer, not legacy envelope) | Find rogue publishers; check NOTIFY payload size and encoding |
| `maidan_bus_notify_hydrate_total{result="ok"}` flat while events post | Listener not receiving NOTIFY or hydrate path bypassed | Check `maidan_bus_listener_ok`; confirm Postgres bus backend |

Subscribers may still recover via event-log replay (`maidan_subscribe_replay_total`);
hydrate drops do not change at-most-once NOTIFY semantics.

### Outbox relay (`v10.0.0` Postgres, `v12.0.0` quarantine, `v14.0.0` SQLite)

Postgres and SQLite deployments enqueue `maidan_outbox` in the same transaction
as `maidan_events`. A background relay publishes after commit (Postgres pointer
NOTIFY; SQLite in-memory bus). HTTP handlers do not call `bus.publish` directly
when relay is enabled.

| Env | Default | Notes |
|-----|---------|-------|
| `MAIDAN_OUTBOX_MAX_ATTEMPTS` | `16` | After this many failed relay publishes, the row is quarantined (`quarantined_at` set). |
| `MAIDAN_OUTBOX_RELAY_MODE` | `notify` | `notify` = `pg_notify` + LISTEN hydrate (multi-instance). `polled` = relay fans out on the process-local bus only (no `pg_notify`). |
| `MAIDAN_OUTBOX_POLL_INTERVAL_MS` | `50` | Base relay poll interval (the **fast** cadence used while draining and right after activity). |
| `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS` | `1000` | Idle-backoff ceiling (`v108.0.0`). When caught up, the relay grows its sleep (×2) up to this cap, then resets to the base interval on the next pending row. |
| `MAIDAN_OUTBOX_RELAY` | `1` (enabled) | Set `0` to disable relay (append-then-publish in-process). **`MAIDAN_ENV=production` rejects `MAIDAN_OUTBOX_RELAY=0`.** |

#### Adaptive cadence (`v108.0.0`)

The relay is adaptive: it **drains back-to-back** (no inter-batch sleep) while a
tick fully relays a batch, so a backlog of N rows clears in ≈⌈N/batch⌉ ticks
instead of N/batch × interval. When caught up it sleeps the base interval and
**backs off** toward `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS` while idle — so a quiet
deployment isn't polling 20×/s for nothing. An **in-process enqueue nudge** wakes
the relay the moment a row is written, so the backoff costs no added latency on a
fresh event (the cap only bounds the worst case if the nudge is ever missed).
Tuning: lower the base interval for snappier single-process fan-out; raise the
cap to poll less when idle. Delivery semantics (at-most-once NOTIFY, quarantine,
replay) are unchanged by cadence.

#### NOTIFY loss / listener unhealthy (`v84.0.0`)

When `maidan_bus_listener_ok` is **0** or `maidan_bus_notify_hydrate_total{result="failed"}` rises but `maidan_outbox_pending` stays high:

1. Confirm the outbox relay task is running (`outbox relay running` in logs; `maidan_outbox_relay_total` incrementing).
2. **Single-process mitigation:** set `MAIDAN_OUTBOX_RELAY_MODE=polled` and restart. Relay publishes to the in-process bus without `pg_notify`. Subscribers on **other** pods still need NOTIFY or WS replay — polled mode is not a multi-instance fan-out replacement.
3. **Multi-instance:** fix LISTEN connectivity (pooler must not pin LISTEN connections; use direct Postgres or a pooler that supports `LISTEN`). Do not disable outbox relay in production.
4. Clients can recover via subscribe replay (`after_id` / `resume_token`) from `maidan_events` while relay catches up.

| Metric | Symptom | Suggested action |
|--------|---------|------------------|
| `maidan_outbox_pending` high | Relay not keeping up or publish failures | Check relay logs; DB connectivity; `maidan_outbox_relay_total{result="failed"}` |
| `maidan_outbox_relay_total{result="failed"}` rising | Bus or hydrate errors during relay | Same as hydrate/bus listener troubleshooting |
| `maidan_outbox_relay_total{result="quarantined"}` | Poison row or persistent bus failure | Inspect row: `SELECT * FROM maidan_outbox WHERE quarantined_at IS NOT NULL`; fix root cause; manual recovery (below) |
| `maidan_outbox_quarantined` > 0 | Unpublished events stopped retrying | Same as quarantined counter |
| `maidan_outbox_oldest_pending_seconds` high | Oldest relayable row aging | Scale relay or fix publish failures before quarantine |
| Events in DB but no live subscribers | Pending rows not relayed | Confirm relay task running; inspect `published_at IS NULL AND quarantined_at IS NULL` |

Relay retries may duplicate NOTIFY; subscribers should dedupe by `log_id`.

### Delivery cursors (`v13.0.0`)

| Surface | Parameter | Notes |
|---------|-----------|-------|
| WebSocket subscribe frame | `consumer_id` | Optional; replay starts above stored cursor |
| MCP `GET /mcp/stream` | `consumer_id` query | Same semantics as WS |

Inspect cursors: `SELECT * FROM maidan_delivery_cursor WHERE workspace_id = $wid;`

Reset a stuck cursor (operator SQL): `UPDATE maidan_delivery_cursor SET last_delivered_log_id = 0 WHERE consumer_id = $id AND workspace_id = $wid;`

**Manual recovery for a quarantined row** (operator SQL, not exposed over HTTP in 12.0):

1. Fix the underlying bus/hydrate issue.
2. **HTTP (`v56.0.0`):** `POST /workspaces/{wid}/outbox/{id}/replay` with `workspace:write` clears quarantine when the row’s event belongs to that workspace.
3. **SQL:** `UPDATE maidan_outbox SET quarantined_at = NULL, attempts = 0 WHERE id = $id;` so the relay picks it up again, **or** leave quarantined and rely on clients replaying from `maidan_events` by `log_id`.

### Automation HTTP delivery (`v68.0.0`)

Slash commands and FSM hooks with `handler_kind: http` enqueue signed POSTs in
`maidan_automation_deliveries`. A background worker retries with exponential backoff;
exhausted rows are quarantined (dead letter). **Outbound event webhooks** still use
`maidan_webhook_deliveries` and `WebhookWorker` — same signing headers, separate queue.
All operator-supplied HTTP destinations use the public-egress guard described
in the environment table above; a target that becomes private after
registration fails delivery rather than being contacted.

| Env | Default | Notes |
|-----|---------|-------|
| `MAIDAN_AUTOMATION_MAX_ATTEMPTS` | `16` | After this many failed HTTP attempts, the row is quarantined. |
| `MAIDAN_AUTOMATION_POLL_INTERVAL_MS` | `50` | Worker poll interval. |

**Dispatch behavior**

| Source | On invoke |
|--------|-----------|
| Slash HTTP | Synchronous POST first; on failure, enqueue and return `retrying` + `delivery_id`. |
| FSM HTTP | Always enqueue; handler returns `{ ok, queued, delivery_id }`. |

**Signing (unchanged from webhooks):** `Content-Type: application/json`, per-registration
`X-Maidan-Event` (or configured header), `X-Maidan-Signature` (HMAC of body), plus
`X-Maidan-Delivery-Id` for idempotency. Integrators must treat delivery as **at-least-once**.

**Operator HTTP** (`workspace:read` / `workspace:write`):

| Route | Use |
|-------|-----|
| `GET /workspaces/:wid/deliveries` | Unified list (`kind=webhook\|automation\|all`, same `quarantined` / `delivered` / `limit` query shape). |
| `GET /workspaces/:wid/deliveries/:did?kind=…` | Single row (`kind` required). |
| `POST /workspaces/:wid/deliveries/:did/replay?kind=…` | Replay webhook or automation DLQ row. |
| `GET /workspaces/:wid/automation/deliveries` | Pending rows (default). Query `?quarantined=1` or `?delivered=1` when supported. |
| `GET /workspaces/:wid/automation/dlq` | Quarantined rows (preferred DLQ list). |
| `GET /workspaces/:wid/automation/deliveries/:did` | Single row. |
| `POST /workspaces/:wid/automation/deliveries/:did/replay` | Clear quarantine and reset attempts for another worker pass. |

| Metric | Symptom | Suggested action |
|--------|---------|------------------|
| `maidan_automation_delivery_total{outcome="failure"}` rising | Targets down or rejecting signatures | Fix endpoint; verify signing secret; inspect `last_error` on row |
| `maidan_automation_delivery_duration_seconds` p95 high | Slow integrator | Tune timeout at integrator; check network |
| Pending rows not draining | Worker not running | Confirm `AutomationDeliveryWorker` spawned in `maidan-server` main |

**Manual recovery (SQL):** `UPDATE maidan_automation_deliveries SET quarantined_at = NULL, attempts = 0, next_attempt_at = datetime('now') WHERE id = $id;` (SQLite) or equivalent `now()` on Postgres — prefer HTTP replay when auth is available.

### Secret substitution on egress

Event webhooks, automation HTTP (slash commands and FSM hooks, the first POST
and every queued retry) and A2A push notifications replace `secret://<name>`
references in their JSON body with the sending workspace's secret value, at
send time, when the delivery URL's host is on that workspace's secret-egress
allowlist (`maidan_secret_egress_hosts`). Any other host gets the literal
reference. Queued rows keep the reference, so a retry after a host is removed
carries the literal; the value is never written to a queue, the event log or an
audit row. Substitution needs `FEDERATION_ENCRYPTION_KEY`; without it every
reference stays literal.

Workspaces manage their own lists (`GET`/`POST /workspaces/:wid/secret-egress-hosts`,
`DELETE …/:host`; MCP `list_`/`allow_`/`revoke_secret_egress_host`). Adding a
host needs `secret:admin` and `secret:read`. The operator's control is a
ceiling:

| Env | Effect |
|-----|--------|
| `MAIDAN_SECRET_EGRESS_ALLOWLIST` | Comma-separated hostnames. Set, a host outside it never receives a value, whatever a workspace lists, and a workspace cannot add it (`400`). Unset, the workspace lists alone decide. Set to the empty string, no host receives a value. Read at boot. |

Narrowing the ceiling does not delete workspace entries outside it; they stay
listed and inert until the ceiling admits them again or a workspace removes
them. Before this list existed the variable was the whole allowlist, for every
workspace; a deployment that set it keeps the ceiling but substitutes nothing
until each workspace lists its hosts.

### Outbound timeouts and the retry budget

Every outbound delivery (event webhooks, automation HTTP, Slack and GitHub
projector and result egress) gives up after 5 s waiting to connect and 10 s in
all, so a receiver that accepts the connection and never answers cannot hold
up the workers. An operator-supplied URL (a webhook, an automation target, a
slash or FSM handler, a federation peer, an A2A push, web push, the advisor)
also gives up after 5 s resolving the name, before those clocks start, so a
nameserver that never answers cannot hold them either. A failed delivery is
retried on its worker's backoff: webhooks and automation wait `2^attempts`
seconds (at most 256 s) up to their
`*_MAX_ATTEMPTS`; mail and egress wait 30 s doubling to an hour, eight attempts.

Backoff spaces one delivery's attempts, not a destination's. When a host that
was down comes back, everything queued for it is due at once. So every worker
in a process shares one **retry budget per destination host** (the webhook or
automation URL's host, `slack.com`, the GitHub API host, the SMTP relay): a
host takes at most 10 retries at once and 2 more a second after that. A first
attempt is never held back and does not count. A retry the budget refuses is
**deferred**, not failed: its next attempt moves 1 to 60 s forward, spaced so
the backlog returns at about the budget's rate, and its attempt count stays as
it was, so waiting never pushes a delivery toward the dead-letter queue. The
limits are constants in `crates/maidan-server/src/retry_budget.rs`, not
environment variables.

The budget is in memory on each replica: with N replicas a recovering host can
take N times the rate, and a restart starts every host's budget full.

| Metric | Meaning |
|--------|---------|
| `maidan_egress_retry_deferred_total{worker}` | Retries the budget deferred, by worker (`webhook`, `automation`, `egress`, `mail`). A sustained rate means a destination is getting retries as fast as the budget allows; it is not a failure count. |

### Result delivery to GitHub and Slack

Result delivery and the GitHub projector post with `MAIDAN_GITHUB_TOKEN`, and
the egress worker runs only when a Slack or GitHub sender is configured: the
GitHub sender needs `MAIDAN_GITHUB_WEBHOOK_SECRET` and `MAIDAN_GITHUB_TOKEN`,
the Slack sender its bot token. They are read from the server's environment,
or from the files `MAIDAN_GITHUB_TOKEN_FILE` and the rest name (see
[Environment](#environment)); set them through your platform's secret
mechanism like every other secret. Maidan never logs the token, and cuts it out of any error text
it records on a delivery or an audit row.

**The change flow** ([Result Delivery](Result%20Delivery.md#the-change-flow-pichangeresult1--github_branch))
commits a coding result to a branch and opens a draft pull request with this
token, so commits and pull requests are authored by the token's owner. Use a
**personal access token** with exactly two
repository permissions, `contents:write` and `pull_requests:write` (in a
fine-grained token: **Contents** and **Pull requests**, read and write), on
exactly the repositories the change flow may write to. Grant nothing else. The flow stops at
a draft pull request: it requests no review and needs no approval (a token
owner cannot approve their own pull request), and the producer marks the pull
request ready. The comment and check-run paths use the same token; a check run
needs `checks:write`, which a personal access token does not carry, so the
check run fails and the comment still posts.

**Guards in Maidan's code, whatever the token can do.** The token may be able
to push anywhere in those repositories, so the limits are enforced by Maidan,
not by GitHub settings, and none of them is configurable:

- The only ref Maidan creates or moves is the target branch, and it must match
  `feature/agent-[a-z0-9][a-z0-9-]*`. `prod`, `main`, `master`, `staging` and
  `dev` are never written. A branch is never its own base.
- `prod` is never a base, for any repository; the allowlist refuses to bless it.
- A pull request is opened only into the base the allowlist names for that
  repository. An open pull request for the branch into any other base refuses
  the change before anything is written.
- Branch moves are fast-forwards (`force: false`). Maidan has no code that
  merges, marks a pull request ready, requests a review, deletes a branch or
  changes repository settings or protection.

A refused change is recorded as `skipped` with the reason on the delivery
(`GET /threads/:id/deliveries`) and said in the Slack thread.

**Seed the allowlist.** Nothing is blessed by default, and nothing seeds it in
code. Each blessing is one audited `token:admin` call,
`POST /workspaces/:wid/egress-targets`, made by an operator per workspace. A
`github_branch` selector is `owner/name@base`: the repository and the one base
change pull requests into it may target (one row per base). A `github` row is
the repository alone and allows result comments only, never commits. For
example, with two repositories, one whose change pull requests target `main`
and one that targets `dev`, and `$SLACK_CHANNEL` the channel id (`C…`) the
replies go to, the seed is:

```bash
H1="Authorization: Bearer $ADMIN_TOKEN"; H2='Content-Type: application/json'
URL="$MAIDAN/workspaces/$WORKSPACE_ID/egress-targets"

# Commits and draft pull requests, one row per repository and base.
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github_branch","selector":"example/skills@main"}'
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github_branch","selector":"example/app@dev"}'

# Result comments on a pull request (the review loop). These allow no commits.
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github","selector":"example/skills"}'
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d '{"surface":"github","selector":"example/app"}'

# The Slack channel the replies go to; a thread_ts needs no row of its own.
curl -sS -X POST "$URL" -H "$H1" -H "$H2" -d "{\"surface\":\"slack\",\"selector\":\"$SLACK_CHANNEL\"}"
```

Each call writes an `egress_target.allow` audit row. `GET
/workspaces/$WORKSPACE_ID/egress-targets` lists the blessings and `DELETE
…/egress-targets/:tid` revokes one; a revoked branch row stops a change that
is already queued, because the allowlist is checked again before the write.

### Agent observability (`v76.0.0`)

Scrape `GET /metrics` for agent-substrate health (see [Integration](Integration.md)). Gate e2e: `agent_substrate_gate_e2e.rs`.

| Metric / signal | Symptom | Suggested action |
|-----------------|---------|------------------|
| `maidan_bus_lag_total` | Subscribers behind | Scope WS filters; scale consumers |
| `maidan_indexer_pending_age_seconds` | Stale embeddings | Fix embedding provider; run `maidan reindex-embeddings` |
| `maidan_outbox_pending` / quarantined | Relay stuck | [Outbox relay](#outbox-relay-v1000-postgres-v1200-quarantine-v1400-sqlite) |
| `maidan_automation_delivery_total{outcome="failure"}` | Slash/FSM HTTP failing | [Automation HTTP delivery](#automation-http-delivery-v6800) |
| MCP tool latency | Not exported per-tool yet | Use HTTP request metrics + logs |

Example Grafana dashboard (Prometheus datasource): `docs/dashboards/maidan-operator.json` (`v89.0.0`).

SLO alert templates (Prometheus / Alertmanager): `docs/alerts/` (`v90.0.0`). CI executes them with promtool via `scripts/check-alert-rules.sh` — the `promtool (alert rules)` required check (`v122.0.0`); run that script locally to validate (it skips with a hint if promtool isn't installed).

**Verify OTLP export end-to-end (`v123.0.0`):** the `otlp` compose profile runs maidan-server against a real OpenTelemetry Collector (`docker/otel-collector-config.yaml`). `./scripts/otlp-smoke.sh` brings up `postgres` + `otel-collector` + a server with `OTLP_ENDPOINT`/`OTLP_METRICS=1`, drives traffic, and asserts the collector received both a traces batch (incl. the per-request `http_request` span) and a metrics batch tagged `service.name=maidan-otlp-smoke`. Run it after touching the OTLP wiring or upgrading the OpenTelemetry SDK. CI runs it as the `otlp smoke` job.

**Semantic scale:** set `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` in Helm prod values; run `maidan reindex-embeddings --database-url $DATABASE_URL` after provider changes.

**Reindex jobs are durable (`v104.0.0`):** `POST /operator/reindex-embeddings` records job status in `maidan_reindex_jobs`, so `GET /operator/reindex-embeddings/:job_id` resolves on any replica and survives restart. The job still *runs* on the replica that started it; if that pod dies mid-run the row stays `Running` — re-issue the (idempotent) reindex. App OAuth codes are likewise durable (`maidan_oauth_codes`): a code minted on one replica is exchangeable exactly once on any replica.

## Search (`GET /workspaces/:wid/search`)

| Query param | Notes |
|-------------|-------|
| `q` | Required search text. |
| `mode` | `lexical` (default) or `semantic` (Postgres + SQLite). |
| `author` / `channel` / `kind` | Optional facets (both modes on Postgres). |
| `limit` | Max hits (default 25). |
| `embedding_model` | Semantic only: registered model name (default: active provider). |

**Semantic mode (`v5.0.0`):** embeds `q` with `MAIDAN_EMBEDDING_PROVIDER`, then queries
the per-model embedding table named by `embedding_model` (default: provider
`model_name()`). Each hit includes `embedding_model`. `/health` reports
`embedding.model` and `embedding.dimension`.

**Rank field:** higher is always better within a single response. Values are
backend-specific for lexical search.

**Score field (`v48.0.0`):** normalized to `[0, 1]` within each response.
Comparable across Postgres and SQLite for the same `mode`. Semantic `score`
is cosine similarity; lexical `score` is min-max normalized `rank`.

| Mode / backend | `rank` meaning | `score` meaning |
|----------------|----------------|-----------------|
| Lexical Postgres | `ts_rank_cd` (unbounded) | min-max normalized rank |
| Lexical SQLite | negative BM25 | min-max normalized rank |
| Semantic (both) | `1.0 - cosine_distance` | same as rank (in `[0, 1]`) |

**Scale:** use Postgres + pgvector HNSW for production semantic search.
Large workspaces should use Postgres.

**SQLite `sqlite-vec` (optional, `v85.0.0`):** `maidan-search` builds without the
extension by default; semantic search on SQLite uses in-process cosine ranking.
Enable SQL `vec_distance_cosine` for dev parity:

```bash
cargo build -p maidan-server --features sqlite-vec
```

CI job `sqlite-vec (optional feature)` proves linkage when the feature is on.

After changing embedding providers, re-index or accept that old-model rows are ignored
until re-upserted under the new model name.

**Operator reindex (`v87.0.0`):** `POST /operator/reindex-embeddings` enqueues a
background job (202 + `job_id`). Poll `GET /operator/reindex-embeddings/:job_id` for
`running` / `completed` / `failed` and `processed` / `failed` counts. Optional JSON
body `{ "workspace_id": "<uuid>" }` scopes to one workspace (`workspace:write`);
omit `workspace_id` for all workspaces (`operator:global`; before, a workspace's `token:admin` could start and read an instance-wide job). CLI `maidan reindex-embeddings`
remains for shell/CI. A job runs in-process on the replica that started it; its record is durable (see below), so a replica that dies mid-run leaves it `Running`.

## Helm (production)

Charts under `helm/maidan` (server) and `helm/maidan-stack` (the server plus an optional
Postgres and MinIO of its own).

| Values file | Use |
|-------------|-----|
| `values.yaml` | Dev defaults (`maidan-server:dev`, a development `DATABASE_URL`) |
| `values-prod.yaml` | The release image, production refusals, HPA + ingress (manual TLS secret) |
| `values-cert-manager.yaml` | Ingress + `cert-manager.io/cluster-issuer` annotation; layer on `values-prod.yaml` |
| `values-profile-otel.yaml` | JSON logs + OTLP traces/metrics (`OTLP_ENDPOINT`, `OTLP_METRICS=1`) |
| `values-profile-redis.yaml` | `MAIDAN_RATE_LIMIT_REDIS_URL` (multi-replica quotas) |
| `values-profile-s3.yaml` | S3-compatible `ARTIFACT_BACKEND` |
| `values-ci.yaml` | kind smoke (SQLite, auth off) |

Layer profiles as needed; see `helm/maidan/PROFILES.md` for example `helm upgrade` commands (`v88.0.0`).

**Production refusals.** `values-prod.yaml` sets `production: true`, and the chart then
refuses to render: a development image (`image.repository: maidan-server`, or a tag of
`dev`, `latest` or empty without `image.digest`); and, unless `existingSecret` names a
Secret holding `DATABASE_URL` and `MAIDAN_CONTENT_KEK`, an unset `secrets.DATABASE_URL`,
the development default `postgres://maidan:maidan@postgres:5432/maidan`, or any empty
`secrets` value. `config` values, `image.tag` and `image.digest` holding `CHANGE_ME` fail every render. `secrets` and `contentKek` values holding `CHANGE_ME` fail when `existingSecret` is unset. Each
refusal names the value to set. `maidan-stack/values-prod.yaml` sets the same flag
(`maidan.production`) and pins the same release.

**cert-manager:** install [cert-manager](https://cert-manager.io/) and a `ClusterIssuer`,
create the `maidan-secrets` Secret (`DATABASE_URL`, `MAIDAN_CONTENT_KEK`), then:

```bash
helm install maidan ./helm/maidan \
  -f ./helm/maidan/values-prod.yaml \
  -f ./helm/maidan/values-cert-manager.yaml \
  --set existingSecret=maidan-secrets \
  -n maidan --create-namespace
```

`values-cert-manager.yaml` alone refuses to render: `values-prod.yaml` is what names the
release image.

**CI validation:** `./scripts/helm-template-smoke.sh` and `./scripts/helm-install-kind-smoke.sh` (kind + Docker).

**Pin by digest.** `image.digest: sha256:…` renders the reference as
`repository@sha256:…`, which a re-pointed tag cannot change; `image.tag` is
then informational. Find a release's digest with
`docker buildx imagetools inspect ghcr.io/david-engelmann/maidan-server:<tag>`,
and verify its signature first (README, "Prebuilt image").

Set `secrets.DATABASE_URL` in values (not a `MAIDAN_` prefix), or name an
`existingSecret` that already holds it. Rendering does not check that Secret
or its keys.

**The umbrella stack's own stores.** `maidan-stack` can run Postgres and MinIO as
single-replica StatefulSets of its own (`postgresql.enabled`, `minio.enabled`;
`values-prod.yaml` enables both):

- Postgres runs `ghcr.io/david-engelmann/maidan-postgres` (`docker/Dockerfile.db`, built on
  `pgvector/pgvector`, so migration 0003's `CREATE EXTENSION vector` works), pinned to the
  same release as the server; the Service is `<release>-postgresql`.
- MinIO runs `cgr.dev/chainguard/minio` by digest, the build compose and `k8s/` use; the
  Service is `<release>-minio`. A post-install and post-upgrade Job creates
  `minio.defaultBuckets` with `cgr.dev/chainguard/minio-client`.
- The server reaches them through a Secret the stack renders, `<release>-datastores`:
  `DATABASE_URL`, and `ARTIFACT_BACKEND=s3` with the `S3_*` settings, overriding
  `maidan.secrets` and `maidan.config`. With `maidan.existingSecret`, the Secret you create
  needs only `MAIDAN_CONTENT_KEK`.
- A production render refuses an empty or development password for either store
  (`postgresql.auth.password`, `minio.auth.rootPassword`) and a Postgres or MinIO image
  without a release tag or digest.

Moving an existing release from the earlier Bitnami subcharts to these StatefulSets is a
replacement, not an in-place upgrade. The StatefulSet selector labels differ (Kubernetes rejects
changing them), and MinIO's volume claim is `data-<release>-minio-0`, not Bitnami's
`<release>-minio`. Dump the database and copy the buckets (`mc mirror`) out, install the new
release, and restore; keep the old volumes until you have checked the restore. Postgres reads
`POSTGRES_PASSWORD` only when it first creates the data directory, so changing it later does not
change the database's password: run `ALTER ROLE` on the live role before changing
`postgresql.auth.password` and upgrading. Changing a store password does not restart the server:
run `kubectl rollout restart deployment/<release>-maidan` after the upgrade so it reads the new
`DATABASE_URL`. A cluster-connected `helm upgrade` restarts MinIO when its root user or password
changes, because the pod template carries a hash of a rollout nonce from the Secret, not of the
password, and that nonce changes only when the live credentials do. `helm template` cannot see
the Secret, so it renders one stable nonce: applying those manifests again does not roll MinIO,
and a credential change there does not either. Restart it with
`kubectl rollout restart statefulset/<release>-minio`.

`minio.persistence.enabled`, `size` and `storageClass` cannot change on an existing StatefulSet.
Kubernetes rejects an update to `volumeClaimTemplates`, and a cluster-connected upgrade refuses
the change before sending it. Copy the buckets out (`mc mirror`), delete the StatefulSet with
`--cascade=orphan`, delete PVC `data-<release>-minio-0` when the new pod needs a new volume, and
upgrade again. The chart README has the steps. Growing a volume, when the storage class allows
it, is a change to that PVC; leave `size` as it is. This is not the Bitnami replacement above.

The stack's Postgres has no replicas, backups or PITR; for those, run your own (or a managed)
Postgres, turn `postgresql.enabled` off and set `DATABASE_URL` as for the server chart. The
install command and the full list are in `helm/maidan-stack/README.md`. The `helm install
(kind)` CI job installs the stack with both stores and waits for `/health/ready`.

## Horizontal scaling (`v105.0.0`)

Maidan runs as **N stateless replicas behind a load balancer** with **no session
affinity** — a request may land on any replica. The `scale` compose profile
(`docker compose --profile scale up`) and the `scale-out smoke` CI job exercise
this with two replicas + an nginx round-robin LB; `scripts/scale-out-smoke.sh`
drives the cross-replica REST paths.

**Shared across replicas (one of each):**

| Resource | Why it must be shared |
|----------|----------------------|
| Postgres (`DATABASE_URL`) | System of record + the `LISTEN`/`NOTIFY` fabric for cross-replica events, presence, and resource notifications. Durable ephemeral state (OAuth codes, reindex job status — `v104.0.0`) lives here too. |
| Object store (`ARTIFACT_BACKEND=s3`) | Artifacts written on one replica must be readable on another. Do **not** use `localfs` with multiple replicas. The client gives up after 5 s to connect and 30 s waiting for the store to start answering (a large body may take longer once it flows), then retries as the AWS SDK does. |
| `MAIDAN_SESSION_SECRET` | Must be **identical** on every replica so subscribe-resume tokens (and session signing) validate regardless of which replica issued them. |

**Still pod-local (do not assume cross-replica):**

- In-flight **MCP streamable sessions** and open WebSocket/SSE subscriptions live on the replica that holds the connection; a reconnect may land elsewhere and resumes from the durable cursor, not in-memory buffer.
- The **outbound retry budget** (10 retries at once, then 2 a second, per destination host) is held per replica, so N replicas give a recovering host N budgets ([Outbound timeouts and the retry budget](#outbound-timeouts-and-the-retry-budget)).
- A **running reindex job** executes on the replica that started it; only its *status* is durable and queryable from any replica. If that replica dies mid-run the row stays `Running` — re-issue the (idempotent) reindex.

**Rolling updates / boot:** every replica runs migrations on boot, serialized by
a Postgres advisory lock (`v105.0.0`) so concurrent starts against a fresh or
upgrading database don't race on DDL. A surge (`maxUnavailable: 0`) starts the
new binary, which migrates, while the previous binary is still serving. That
overlap follows [Migrations](Migrations.md): the migration that runs then only
expands, and a drop, rename, retype, or rewrite is a later contract, after the
previous binary is gone. A migration that cannot expand is a cutover: stop the
previous binary before the new one migrates. There is no HTTP or MCP
compatibility promise before the product's own 1.0 gate (Decisions, F-54).
`/health/ready` gates traffic on the database, the object store, the indexer,
and the `LISTEN` bus, so a load balancer that honors readiness does not route
to a replica mid-migration.

**Not covered:** load/throughput benchmarking (bench harness),
autoscaling/HPA tuning, multi-region active-active (out of scope).

## Read replicas (`v264.0.0`)

Maidan can offload reads to a Postgres streaming **read replica**, with a causality
token that guarantees a client never reads staler than its own writes.

**Enable it.** Set `MAIDAN_DB_REPLICA_URL` to a hot-standby's connection string.
The server connects it at boot (fail-fast on a bad URL) and a background task polls
the standby's replay position every 200 ms, so each read's primary-vs-replica choice
is a cheap in-memory compare (no extra round-trip). Unset → every read uses the
primary (unchanged).

**The consistency token.** A successful mutating request returns a
`Maidan-Consistency-Token` response header (the primary's WAL LSN at that point). A
client that wants read-your-writes echoes it on a later request as the
`Maidan-Consistency-Token` request header. That read is served from the replica only
once the replica has replayed past the token; until then it falls back to the
primary. A read with no token may be served from the replica immediately (the caller
has asserted no causality requirement).

**Not `Maidan-Room-LSN`.** That header is **your workspace's** event-log
high-water (decimal, including SQLite) so a subscriber or webhook consumer can
see projector / broadcast lag. It is scoped to the caller's
room rather than the instance — the instance head was not comparable to anything
a client had seen, so a caught-up consumer could never reach it. It is
**not** a WAL LSN, is not gated on a replica, and must not be echoed as
`Maidan-Consistency-Token`. See [Integration.md](Integration.md) (subscribe).

**What routes, and what never does.**

- **Routed** (only for `GET`/`HEAD`): content and collaboration reads — messages,
  threads, channels, members, DMs, social (votes/reactions/pins/mentions),
  notifications, follows, skills, assignments, dependencies, queue depth, and usage.
  **Message search** (`v271.0.0`) routes too: `maidan-search`'s `PostgresSearch` has
  its own replica reader pool + replay poller and honors the same token via the same
  routing logic, so `GET …/search` reads-your-writes and offloads to the replica
  identically (embedding writes / index DDL / reindex stay on the primary). Its
  primary/replica split is counted separately as `maidan_search_replica_reads_total`
  (`v272.0.0`).
- **Always the primary:** every write; **auth-path reads** (sessions, API tokens,
  OIDC, federation peers) — the auth middleware runs on `GET`s, so a just-minted
  credential must be read fresh; **control-plane/config reads** (webhooks, slash
  commands, FSM hooks, deliveries, reindex jobs, audit, token quotas); and any read
  inside a mutation handler (those requests are never in a read-routing scope, so a
  read-then-write decision is always on primary data).

**Observability.** `maidan_replica_reads_total{outcome="primary"|"replica"}` counts
the store split and `maidan_search_replica_reads_total{outcome}` (`v272.0.0`) the
search split; `maidan_replica_lag_bytes` is the replica's WAL lag (primary write LSN
minus replica replay LSN, shared by both). Complement with Postgres's own
`pg_stat_replication`.

**Testing.** `scripts/replica-harness.sh up` stands up a local pgvector primary +
streaming standby and prints `MAIDAN_PRIMARY_URL` / `MAIDAN_REPLICA_URL`; the
`#[ignore]`d `read_routing` / `replication` store tests validate store routing and
read-your-writes against it (`cargo test -p maidan-store --test read_routing --
--ignored`), and the `#[ignore]`d `replica_routing` search test proves the same for
message search (`cargo test -p maidan-search --test replica_routing -- --ignored`).

## Event-log chain verification

Every event carries a `prev_hash` / `content_hash` chain. Verifying
it end to end is `GET /workspaces/:wid/events/verify`, which is
streaming rather than whole-log-in-memory.

The search tap used to re-walk and re-verify the entire log on
every process start — which meant verification happened only when a process
happened to bounce. That is not a control you can schedule, alert on, or say
when it last ran, so the tap now resumes from a cursor and verification became
an explicit job:

```sh
MAIDAN_CHAIN_VERIFY_SECS=86400   # daily; unset = off
```

It walks every workspace that has events, continues past a break so one tenant's
tamper does not hide the others, and emits
`maidan_chain_verify_total{outcome="ok"|"broken"|"error"}`.

**Alert on `broken`, investigate `error` separately.** `broken` means a chain
verified and failed — a tamper or corruption. `error` means the verification
could not run, which is usually a database problem. An alert that cannot tell
them apart gets ignored.

A broken chain is logged with its `workspace_id`, `break_at` and reason; the
recovery path is a rebuild from the domain tables, not a repair of the log.

## Backup & disaster recovery (`v260.0.0`)

Maidan's durable state is two things, and the backup story follows the same split:

| What | Store | Backed up by |
|------|-------|--------------|
| System of record — every workspace, member, channel, thread, message, event log, audit trail, token, follow/pref/schedule | **Postgres** or **SQLite** (`DATABASE_URL`) | `pg_dump -Fc`; SQLite `VACUUM INTO` |
| Content-addressed artifact blobs (immutable, deduped) | `localfs` root **or** an object store (`ARTIFACT_BACKEND=s3`) | a tar of the localfs root; for S3 the bucket itself is the durable copy |

Two operator scripts implement it:

- **`scripts/backup.sh [BACKUP_DIR]`** — `pg_dump` (custom format), or for
  SQLite a `VACUUM INTO` snapshot (below), plus, for
  `localfs`, a `tar` of `ARTIFACT_LOCALFS_ROOT`; writes a `MANIFEST.txt`. For
  `s3`, the bucket is the durable copy — enable **bucket versioning** and/or
  cross-region replication there rather than copying blobs into the backup.
- **`scripts/restore.sh <backup-dir> [--force]`** — `pg_restore` into the target
  `DATABASE_URL`, or for SQLite the snapshot put in place of the file (+ untar artifacts). It **refuses a non-empty target** unless
  `--force`, so a restore can't silently clobber a live database; `--force` restores
  with `--clean --if-exists`.

**Not in the data backup — restore these from your secret manager, out of band:**
`DATABASE_URL`, `MAIDAN_SESSION_SECRET` (subscribe-resume/session signing),
`FEDERATION_ENCRYPTION_KEY` (+ any `FEDERATION_DECRYPT_KEYS` — see the
rotation keyring), `MAIDAN_EXPORT_SIGNING_KEY` (and any
`MAIDAN_EXPORT_VERIFY_KEYS` pin), `MAIDAN_CONTENT_KEK` (+ any `MAIDAN_CONTENT_KEK_PREVIOUS`; without it no message words can be read), and SMTP/OIDC credentials. A DB dump without the session secret
still restores all data; only signed-token continuity needs the same secret.

**RPO / RTO.** A periodic `backup.sh` (e.g. hourly cron) gives an RPO of one backup
interval. For a tighter RPO, run Postgres with **WAL archiving / PITR** (below, or a
managed Postgres with continuous backup) — the logical dump is the portable floor,
not the lower bound. RTO is a `restore.sh` run plus a `/health/ready` check before the load
balancer is pointed at the restored instance.

**Recovery outline.** Provision Postgres + the artifact store → set the out-of-band
secrets → `DATABASE_URL=… ARTIFACT_LOCALFS_ROOT=… scripts/restore.sh <dir> --force`
→ start one replica and confirm `/health/ready` is `200` (it gates on DB + object
store + indexer + the `LISTEN` bus) → scale out. Because artifacts are
content-addressed, a message referencing a blob that predates the artifact backup is
still consistent after restore; a blob written *after* the last artifact archive is
the only thing a stale artifact backup can miss.

### SQLite

Do not copy a live SQLite file. In WAL mode (which Maidan sets) recent writes
sit in `maidan.db-wal` until a checkpoint, so a copy of the file alone misses
them, and a copy taken during a write can catch a torn page.

**Back up** with the server running: `DATABASE_URL=sqlite:///data/maidan.db
scripts/backup.sh` runs `VACUUM INTO`, which writes a consistent, compacted
snapshot (`maidan.sqlite`) from one read transaction, waiting out a write in
progress, and checks it with `PRAGMA integrity_check`. It needs the `sqlite3`
CLI. Take it on a schedule; the RPO is the interval, as for `pg_dump`.

**Restore** with the server stopped: `DATABASE_URL=sqlite:///data/maidan.db
scripts/restore.sh <backup-dir> --force` checks the snapshot, puts it in place
of the file, and deletes the old `-wal` and `-shm`. Deleting them is not
housekeeping: a `-wal` left from a server that was killed (or beside a file
someone removed) is replayed over whatever file has that name the next time it
is opened, and the restored database comes back as the old one, or corrupt.
Without `--force` it refuses a target that already has tables, or that is not
a readable SQLite database; with `--force` it does not open the target, so it
replaces a corrupt file too. The restored file keeps the owner and mode of the
file it replaces. A new target belongs to whoever runs the script, so run it as
the server's user (or `chown` the file after). A path in `DATABASE_URL` is
percent-decoded, as the server decodes it (`%3F` is a `?` in the file name).
Start the server, confirm `/health/ready`, and it migrates forward if the
snapshot is older than the binary.

**The drill.** `scripts/sqlite-backup-drill.sh` takes a snapshot while a writer
is inserting, then restores it over a killed server's database, over an
orphaned `-wal` and over a file that is not a database, and to a
percent-encoded path, and fails unless exactly the snapshot's rows come back.
`sqlite_backup` (a store test) shows a snapshot of a migrated Maidan database
passes `integrity_check` and `foreign_key_check` and opens and migrates as it
stands. CI runs the drill as `sqlite backup drill`.

### Point-in-time recovery

With WAL archiving on, a base backup plus the archive restores the database to
any moment since that backup, not only to the last dump.

**Enable it.** Set `wal_level=replica`, `archive_mode=on` and an
`archive_command` that copies each finished segment somewhere durable, and set
`archive_timeout` so a quiet database still closes a segment at least once a
minute. `compose.pitr.yaml` does this for the compose stack, archiving to a
volume. In production, point `archive_command` at storage off the database host,
through WAL-G, pgBackRest or a managed service: an archive on the same disk
survives a bad deploy, not a lost disk. Take a base backup
(`pg_basebackup -X none -c fast`) when you enable it and on a schedule; recovery
replays WAL from the most recent one.

**Restore to a moment.**

1. Stop the server and put the base backup in a fresh data directory.
2. Create `recovery.signal` in it.
3. Start Postgres with `restore_command = 'cp /archive/%f %p'` (or your tool's
   fetch command), `recovery_target_time = '<timestamp with zone>'` and
   `recovery_target_action = promote`.
4. When `SELECT pg_is_in_recovery()` returns `false`, the database is at the
   target and writable. Start one replica, confirm `/health/ready`, then scale
   out.

Artifacts are content-addressed, so a restored message still names the blob it
had. Restore the artifact store to a point no earlier than the database target.

**The drill.** `scripts/pitr-drill.sh [image]` runs this whole procedure with
Docker. It writes a row, notes the time, writes another, restores to that time,
and fails unless exactly the first row is back. CI runs it on every PR against
the `maidan-postgres` image built from the tree.

## Signed workspace export

A **tenant portability** file is not a `pg_dump`. `GET /workspaces/:id/export`
(`token:admin`) writes a `maidan.workspace.export/1` Ed25519 envelope a
fresh GHCR instance can verify with `POST /workspaces/export/verify` and
import with `POST /workspaces/import` — no callback to the origin host.

**Tokens die on export.** The bundle omits API tokens and secrets. After
import, mint new tokens (`token:admin`). A `pg_dump` restore *does*
preserve hashed tokens (same `DATABASE_URL` / same instance); a signed
export is a *different* machine and must not.

Set `MAIDAN_EXPORT_SIGNING_KEY` on the origin. On a destination that
should accept *only your* key, set `MAIDAN_EXPORT_VERIFY_KEYS` to that
public key (`GET /operator/export-public-key` on the origin). A blank
instance with neither key still verifies integrity (tamper-evident).
See [Integration.md](Integration.md#workspace-portability-signed-export).

## Crypto-shredding

A message's words (body, metadata, content blocks) are encrypted with a key
of their own (XChaCha20-Poly1305) before the event is hashed. Withdrawing the
message destroys that key, so the words are gone from the event log, admin and
peer catch-up, exports, snapshots and search, while the hash chain and
signatures still verify. Workspace purge destroys every key in the workspace;
its audit row counts them (`metadata.content_keys_destroyed`). A replica that
ingests the origin's tombstone shreds its copy. Under a legal hold the preserved
copy keeps the words, as described below.

Content keys live in `maidan_content_keys`, each wrapped by `MAIDAN_CONTENT_KEK`.
The server refuses to start without the KEK. The Helm chart refuses to render
without `contentKek` or an `existingSecret` holding `MAIDAN_CONTENT_KEK`, and the
k8s base reads it from `maidan-secrets`. Only an explicit
`MAIDAN_ALLOW_INSECURE_DEV_KEK=1` (the compose files and local development set
it) falls back to the built-in development key, which is public; production
refuses that flag.

A withdrawal also deletes the notification mail about the message, sent or
pending, and a notification about a withdrawn message is never queued. (Mail
bodies name only the notification kind and event number, never the words; a
send already in flight completes.)

**Check for leftovers.** `maidan verify-shredding --database-url …
[--workspace-id …]` reads every withdrawn message and lists any copy of its
words still outside the sealed event log: a message row not blanked, earlier
versions (kept on purpose under a legal hold), an unsealed event payload, a
queued webhook, egress or mail copy, a search entry or an embedding. It exits
non-zero when it finds one.

**Rotate the KEK.** Generate a new key (`openssl rand -hex 32`), set it as
`MAIDAN_CONTENT_KEK`, move the old one to `MAIDAN_CONTENT_KEK_PREVIOUS`, and
roll the replicas. Each rewraps the keys still under an old KEK at startup.
Remove the old KEK once no key needs it:

```sql
SELECT count(*) FROM maidan_content_keys
 WHERE wrapped_key IS NOT NULL AND kek_id <> '<new kek id>';
```

The KEK id is logged at startup. A key wrapped by a KEK the server does not
have fails the read with a 500; it is never shown as withdrawn.

**Backups.** A database backup taken before a withdrawal still holds that
message's key. Anyone with that backup and the KEK can read the words. Keep the
KEK out of data backups, and expire backups within your erasure deadline.

**Artifacts.** Artifacts are deduplicated across workspaces. `DELETE
/artifacts/{sha}` (`token:admin`, audited as `artifact.erase`) removes the
calling workspace's reference; the bytes are deleted only when the last
workspace lets go (`last_reference: true`). An upload of the same bytes
waits for an erase in progress and then writes the bytes back, so a new
reference never points at deleted bytes; workspace purge uses the same
protocol.

## Legal holds

A legal hold stops a workspace's data from being destroyed while litigation is
pending or reasonably anticipated (FRCP 37(e); GDPR Art. 17(3)(e) exempts such
data from erasure). Holds are per matter: place one for each, and lift each on
its own, as a workspace admin (`token:admin`):

```http
POST /workspaces/{id}/legal-holds
{"reason": "matter 2026-114"}
GET /workspaces/{id}/legal-holds
DELETE /workspaces/{id}/legal-holds/{hold_id}
```

While a workspace has any hold:

- workspace purge and erase, message purge, artifact erase, and an import that
  replaces the workspace are refused (409), inside the destroying transaction;
- its event-log rows and its audit rows are exempt from retention pruning,
  and its own retention policy prunes nothing. Other workspaces' rows, and
  instance-level audit rows, still prune;
- **a withdrawn message keeps its words.** When a member (or a moderator)
  tombstones a message, it disappears for everyone exactly as it would unheld,
  but its last body and every earlier version are kept. Nothing in the product
  tells the member the words were kept.

Read what the holds kept with `GET /workspaces/{id}/legal-holds/preserved`
(`token:admin`): one entry per withdrawn message, with its author, thread,
channel, times, last words and earlier versions. Each read writes a
`legal_hold.preserved_read` audit row first; if the row cannot be written,
nothing is returned. Listing the holds is `token:admin` too: the members a hold
binds are not shown that it exists.

Lifting one matter's hold releases nothing while another stands. Lifting the
last disposes of what the holds kept: the preserved bodies, and the earlier
versions of messages withdrawn while held. That lift's audit row records how
many of each (`metadata.disposed`). Without a hold, a withdrawal takes the
message's earlier versions with it.

## Retention

Nothing is pruned unless you set a retention, or a workspace sets its own. Each
knob is an age in days; the sweeper runs every `MAIDAN_RETENTION_SWEEP_SECS`
(default 86400) and deletes in batches of `MAIDAN_RETENTION_BATCH` (default
5000), so a first sweep over a long-unpruned table does not lock it. Each sweep
counts what it deleted in `maidan_retention_pruned_total{table}` (`events`,
`audit`, `notifications`, `deliveries`, and `messages`).

| Variable | What it prunes | What it never prunes |
|---|---|---|
| `MAIDAN_RETENTION_EVENTS_DAYS` | Event-log rows older than the cutoff | Anything above the lowest at-least-once delivery cursor still advancing, so a lagging consumer loses nothing undelivered; a held workspace's events |
| `MAIDAN_RETENTION_AUDIT_DAYS` | Audit rows older than the cutoff, including instance-level rows (no `workspace_id`) | A held workspace's audit rows |
| `MAIDAN_RETENTION_NOTIFICATIONS_DAYS` | Read notifications older than the cutoff | Unread notifications; any notification with a snooze set, including one whose snooze has lapsed; a held workspace's notifications |
| `MAIDAN_RETENTION_DELIVERIES_DAYS` | Finished delivery rows: delivered or quarantined webhook and automation deliveries, published transactional-outbox rows, delivered projector and result egress, delivered notification mail, and dead-lettered agent runs | Pending or retrying rows, whatever their age; egress and mail **dead letters**, which leave when an operator requeues them (`MaidanEgressDeadLettered` and `MaidanMailDeadLettered` fire while any exist); a held workspace's rows in every one of these tables |
| `MAIDAN_RETENTION_MESSAGES_DAYS` | Messages posted before the cutoff, erased the way a purge erases them (words, embeddings, references, content keys) in every workspace | Messages in a held workspace; a message newer than the cutoff. Off when unset |

Audit rows belong to the workspace stamped on them when they were written. On
upgrade, migration 0123 backfilled older rows from what they reference: a
`workspace` target, `metadata.workspace_id`, the target's own row (token,
member, channel, thread, message, share ticket, grant, egress target, app
installation, result delivery, reindex job, hold), then the actor's or
subject's membership. A row none of those resolved (its members and target
since erased) has no workspace: it is instance-level, shown only in
`GET /operator/audit`, and pruned under the instance cutoff whatever holds
stand. Count them with
`SELECT count(*) FROM maidan_audit WHERE workspace_id IS NULL`.

`MAIDAN_RETENTION_NOTIFICATIONS_DAYS` is off when unset. It deletes read
notifications older than that many days and leaves unread ones, snoozed ones
and a held workspace's rows. The usage ledger is not pruned.

A delivered egress row goes with its dedup key, so the same source event
could be queued again only if the router replayed an event older than the
cutoff. It does not: it starts from the log head and replays only across a
subscriber lag. The external comment or message a result delivery
edits in place is tracked on `maidan_result_deliveries`, which retention does
not touch.

### Per-workspace retention

A workspace administrator can set a shorter retention for that workspace's
messages, events and finished deliveries with `PUT /workspaces/{wid}/retention`
(`token:admin`, audited; see [Integration](Integration.md#workspace-retention)).
The instance knobs above are the ceiling: a workspace value longer than the
instance's for that kind is refused, and if you later lower an instance knob
below a workspace's value, the instance's applies. `MAIDAN_RETENTION_MESSAGES_DAYS`
is that ceiling for messages. When it is set, the instance sweep erases messages
past it in every workspace that is not held, whether or not the workspace set a
policy. When it is unset, only a workspace policy prunes messages, and that
policy may be any value from 1 to 3650 days. A workspace may set a shorter
`messages_days` and not a longer one. After the instance sweep, each sweep prunes
every workspace that set a stricter policy past its own cutoff; a held workspace
keeps everything. Message pruning erases a message the way a purge does, including
its embeddings, references and content key. Policies are stored in
`maidan_retention_policies` (migration 0134) and removed with their workspace.

## Event-log hash chain

Every stored event is accompanied by `{id, lsn, prev_hash, content_hash}`
(SHA-256, `sha256:<hex>`). `lsn` is the event-log `id`, not a WAL
`Maidan-Consistency-Token`. The chain is hashed, not signed: a peer
that has seen a prefix detects a splice or payload rewrite without
trusting the host. Authorship of a wholly fabricated but consistent
chain is the signed-export envelope above.

`GET /workspaces/:wid/events/verify` (`workspace:read`) walks the
retained suffix and **409s** (`event-log-broken`) on a break. Federation
ingest checks origin hashes the same way before remap. After retention
prune, verify the remaining suffix — snapshot catch-up of a dropped
prefix is described below.

No extra env vars. See [Integration.md](Integration.md#event-log-hash-chain)
and [Threat-Model.md](Threat-Model.md).

## Log snapshot

A **log snapshot** (`GET /workspaces/:id/snapshot`) is a
different artifact: a hashed checkpoint of the same content graph plus
the retained event-log floor/head, so a peer can catch up after
retention pruned the prefix. It is not Ed25519-signed (391 is
authorship). `include_graph=true` is `token:admin` or a federation
peer — the default header + `graph_hash` is enough to verify a later
graph fetch. See [Integration.md](Integration.md#snapshot--since-lsn-catch-up).

## API stability

From `v1.0.0`, HTTP and MCP shapes are semver-stable. Pre-1.0 releases
may break without migration shims.
