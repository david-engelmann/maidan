# Configuration reference

Every setting the server reads, grouped by area. [Self-hosting
Maidan](Production.md) says which ones a deployment needs and how to set them.

## How configuration works

- **Environment variables only.** The server and the `maidan` CLI read their
  configuration from the environment at boot. There is no config file.
- **Unknown names refuse boot.** A `MAIDAN_*` variable the server does not know
  stops startup and names the variable it probably meant, so a typo cannot
  silently leave a default in place. `MAIDAN_ALLOW_UNKNOWN_ENV=1` starts anyway
  and logs the unknown names. In a Kubernetes pod the service-link variables
  Kubernetes injects for a Service named `maidan-…` (`MAIDAN_…_SERVICE_HOST`,
  `MAIDAN_…_PORT_8080_TCP` and the rest) are tolerated; the Helm chart and
  `k8s/` turn them off with `enableServiceLinks: false`.
- **An empty value counts as unset.**
- **Secrets can come from files.** Each secret below has a `<NAME>_FILE` form
  that names a file holding the value, such as a Docker or Kubernetes secret
  mount, so the container's config shows a path and not the secret. The file
  is read once at boot, with trailing newlines trimmed. Setting both `<NAME>`
  and `<NAME>_FILE` refuses boot, as does a file that cannot be read, is empty
  or is not UTF-8. The error names the variable and the path, never the value.
  The server logs which variables came from files.

The secrets with a `_FILE` form: `DATABASE_URL`, `MAIDAN_DB_REPLICA_URL`,
`MAIDAN_RATE_LIMIT_REDIS_URL`, `MAIDAN_CONTENT_KEK`,
`MAIDAN_CONTENT_KEK_PREVIOUS`, `MAIDAN_SESSION_SECRET`,
`MAIDAN_SUBSCRIBE_RESUME_SECRET`, `MAIDAN_OIDC_CLIENT_SECRET`,
`MAIDAN_TURNSTILE_SECRET`, `MAIDAN_EXPORT_SIGNING_KEY`,
`MAIDAN_GITHUB_APP_PRIVATE_KEY`, `MAIDAN_GITHUB_TOKEN`,
`MAIDAN_GITHUB_WEBHOOK_SECRET`, `MAIDAN_DEPLOY_HOOK_SECRET`,
`MAIDAN_SLACK_BOT_TOKEN`, `MAIDAN_SLACK_SIGNING_SECRET`,
`MAIDAN_LINEAR_API_KEY`, `MAIDAN_SMTP_PASSWORD`, `MAIDAN_EMBEDDING_API_KEY`,
`MAIDAN_VAPID_PRIVATE_KEY`, `FEDERATION_ENCRYPTION_KEY`,
`FEDERATION_DECRYPT_KEYS`, `S3_ACCESS_KEY_ID` and `S3_SECRET_ACCESS_KEY`. A
test fails when the server starts reading a new secret that has no `_FILE`
form.

In the tables, "boot refuses" means the server exits at startup with an error
naming the variable.

## Core

| Variable | Default | Notes |
|---|---|---|
| `DATABASE_URL` | required | `postgres://…` (recommended) or `sqlite:…`; the prefix picks the backend. A managed Postgres (Neon, RDS, Cloud SQL and the like) needs TLS: add `?sslmode=require`, or `sslmode=verify-full` to check the certificate. The server carries its own root certificates, so this works on the shell-less image. SQLite connections turn on `foreign_keys`, WAL and a busy timeout. |
| `MAIDAN_BIND` | `0.0.0.0:8080` | The address the HTTP server listens on. |
| `MAIDAN_ENV` | unset | `production` refuses every development switch: `AUTH_DISABLED`, the development KEK, `MAIDAN_OIDC_MOCK`, `MAIDAN_ALLOW_PRIVATE_EGRESS`, `MAIDAN_OUTBOX_RELAY=0`, `MAIDAN_OPS_FAULT` and the anonymous MCP reader. |
| `MAIDAN_PUBLIC_ORIGIN` | unset | This instance's public origin, such as `https://maidan.example.com` (`https`, or `http` on a loopback host, with no path). OAuth identifiers are built from it; unset, the instance serves no OAuth metadata. A malformed value refuses boot. See [OAuth](OAuth.md). |
| `MAIDAN_CONSOLE_ORIGIN` | origin of `MAIDAN_OIDC_REDIRECT_URI` | The console's absolute origin, used in the one-time links `approval_decide` gives a person. With neither set, the links are host-relative and no URL elicitation is offered. |
| `MAIDAN_SHUTDOWN_DRAIN_SECS` | `0` | Seconds the server keeps serving after SIGTERM, with readiness at `503`, before it closes its listener. The Helm chart and `k8s/base` set `10`. See [Rolling restarts](Production.md#rolling-restarts). |
| `MAIDAN_LOG` | `info,sqlx=warn` | A `tracing` filter for the server and the CLI. |
| `MAIDAN_LOG_FORMAT` | `plain` | `json` for one JSON object per line. |
| `MAIDAN_ALLOW_UNKNOWN_ENV` | unset | `1` starts despite unknown `MAIDAN_*` variables and logs them. |

## Encryption and signing keys

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_CONTENT_KEK` | required | 32-byte key-encryption key (64 hex characters or standard base64). It wraps each workspace's key, which wraps each message's key ([Crypto-shredding](Production.md#crypto-shredding)). The server and `maidan init` refuse to start without it. Generate one with `openssl rand -hex 32` and keep it in a secret manager, never in data backups. |
| `MAIDAN_CONTENT_KEK_PREVIOUS` | unset | Comma-separated retired KEKs that may still unwrap, during a rotation. Needs `MAIDAN_CONTENT_KEK`. |
| `MAIDAN_ALLOW_INSECURE_DEV_KEK` | unset | `1` lets a server with no KEK use the built-in development key, which is public, with a warning. Local development and CI only; refused in production. |
| `MAIDAN_WORKSPACE_KEY_CUSTODY` | `local` | Who wraps each workspace's key: `local` (the instance KEK) or `aws-kms`. Keys the old custody wrapped keep opening until the rewrap pass has moved them. An unknown value refuses boot. |
| `MAIDAN_KMS_INSTANCE` | unset | With `aws-kms`: a name unique to this instance in the AWS account. See [AWS KMS custody](#aws-kms-custody). |
| `MAIDAN_EXPORT_SIGNING_KEY` | unset | 32-byte Ed25519 seed (hex or base64) that signs workspace exports. Export refuses until it is set; there is never an unsigned bundle. Losing it does not strand existing files, which carry the public key. |
| `MAIDAN_EXPORT_VERIFY_KEYS` | unset | Comma-separated public keys. Set, import and verify accept only bundles signed by them. Unset, a bundle is checked against its embedded key only. |
| `FEDERATION_ENCRYPTION_KEY` | unset | 32-byte key (hex or base64) that encrypts federation peer tokens, A2A push credentials and workspace secrets at rest. Needed to create peers and push configs and to substitute `secret://` references. Back it up with your secrets; rotating it means re-creating peers. |
| `FEDERATION_DECRYPT_KEYS` | unset | Comma-separated retired keys that may still decrypt, during a rotation of `FEDERATION_ENCRYPTION_KEY`. |

### AWS KMS custody

With `MAIDAN_WORKSPACE_KEY_CUSTODY=aws-kms`, each workspace key gets its own
KMS key. The server needs `CreateKey` (tagged `maidan_workspace_key=<id>`),
`TagResource`, and `Encrypt` and `Decrypt` with the workspace key id as
encryption context. On rotation or erasure it calls `DisableKey` and then
`ScheduleKeyDeletion` (seven-day window), so an erased workspace cannot be
opened from any backup. Credentials and region come from the AWS chain:
environment variables, web identity, SSO, profile static keys, ECS and EC2
instance roles. `credential_process` profiles are not supported.

With `MAIDAN_KMS_INSTANCE` set, each key is also tagged
`maidan_instance=<name>`. An hourly sweep (needing `ListKeys`,
`ListResourceTags` and `DescribeKey`) schedules deletion of this instance's
enabled keys older than a day that this database journalled a wrap for and no
workspace key refers to: the leftovers of a wrap whose transaction never
committed. A key this database never journalled is never destroyed, so a
second deployment that reuses the name (a staging copy restored from a
production snapshot) cannot destroy the first one's keys. On SQLite the
journal is in memory, so keys left by a wrap before a restart are kept.
Unset, keys carry no instance tag and nothing is swept. Metrics:
`maidan_custody_stray_keys_destroyed_total`,
`maidan_custody_stray_keys_failed` and `maidan_workspace_keys_undestroyed`.

## Authentication and sessions

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_SESSION_SECRET` | required with auth on | At least 32 bytes. Signs browser session cookies and subscribe resume tokens; must be the same on every replica. Required when OIDC is on. With OIDC off, `MAIDAN_SUBSCRIBE_RESUME_SECRET` can stand in for it; the console then makes no session from a pasted token and keeps the token in the tab. |
| `MAIDAN_SESSION_TTL_SECS` | `28800` | Browser session lifetime (8 hours), OIDC or token. A token's session also ends with the token. |
| `MAIDAN_SUBSCRIBE_RESUME_SECRET` | `MAIDAN_SESSION_SECRET` | A separate key for subscribe resume tokens only. |
| `MAIDAN_SUBSCRIBE_RESUME_TTL_SECS` | `3600` | Resume token lifetime. |
| `MAIDAN_COOKIE_SECURE` | unset | `1` marks session cookies `Secure`. Production does this already. |
| `AUTH_DISABLED` | unset | Serve every request unauthenticated. Takes effect only with `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` as well, and never with `MAIDAN_ENV=production`; either violation refuses boot. Development, tests and CI only. |
| `MAIDAN_ALLOW_INSECURE_NO_AUTH` | unset | The acknowledgement `AUTH_DISABLED` needs. Never set it in production. |
| `MAIDAN_BOOTSTRAP` | unset | `1` opens unauthenticated `POST /workspaces` and `POST /workspaces/:wid/members`, only on a build with the `bootstrap` feature. The published server image has no such routes, so it does nothing there. See [HTTP bootstrap](Production.md#http-bootstrap-development-only). |
| `MAIDAN_ALLOWED_HOSTS` | unset | Comma-separated host names (no ports) that a request with no credential may name under `AUTH_DISABLED` or the anonymous MCP reader, besides loopback names and IP addresses. See [DNS rebinding](Production.md#dns-rebinding). |
| `MAIDAN_DEV_ANONYMOUS_MCP_WORKSPACE` | unset | Development only. A workspace id whose name begins `synthetic-`, which an MCP `POST` with no credential may read with read-only tools. Refused in production and beside `AUTH_DISABLED`. |

## OIDC sign-in

The design and provider recipes are in [OIDC](OIDC.md).

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_OIDC_ENABLED` | unset | `1` turns on `/auth/oidc/*` and the session routes. |
| `MAIDAN_OIDC_ISSUER` | required with OIDC | The identity provider's issuer URL, for discovery. |
| `MAIDAN_OIDC_CLIENT_ID` | required with OIDC | The OAuth client id. |
| `MAIDAN_OIDC_CLIENT_SECRET` | unset | The code-exchange secret, for a confidential client. |
| `MAIDAN_OIDC_REDIRECT_URI` | required with OIDC | The registered callback, such as `https://maidan.example.com/auth/oidc/callback`. |
| `MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI` | unset | The registered post-logout redirect, used when the provider has an `end_session_endpoint`. |
| `MAIDAN_OIDC_SCOPES` | `openid profile email` | Space-separated scopes. |
| `MAIDAN_OIDC_PENDING_TTL_SECS` | `600` | How long a login may take. |
| `MAIDAN_OIDC_FIRST_ADMIN` | on | A session may mint the workspace's first `token:admin` with `POST /auth/session/mint`. `0` turns it off. |
| `MAIDAN_OIDC_AUTO_MINT` | unset | `1` sends a person to `/ui/?auto_mint=1` after login when the workspace has no `token:admin` yet. Needs first-admin mint. |
| `MAIDAN_OIDC_AUTO_PROVISION` | unset | `1` lets a new identity create a human member. Otherwise the identity must already be provisioned or linked. |
| `MAIDAN_OIDC_LINK_EMAIL` | unset | `1` links a verified email claim to an existing member with that handle. |
| `MAIDAN_OIDC_EMAILS_VERIFIED` | unset | `1` treats an `email` claim without `email_verified` as verified. Only for a provider whose every login proves the address. An explicit `email_verified: false` stays unverified. |
| `MAIDAN_OIDC_MOCK` | unset | `1` for a deterministic development and CI provider. Refused in production. |

## Sign-up and waitlist

Off unless `MAIDAN_SIGNUP=1`. [OIDC](OIDC.md) describes the flow.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_SIGNUP` | unset | `1` turns on the waitlist and invited sign-up at `/auth/oidc/signup`. Off, the whole surface answers 404. Boot refuses it beside `MAIDAN_OIDC_AUTO_PROVISION`. |
| `MAIDAN_SIGNUP_EMAIL_DOMAINS` | unset | Comma-separated email domains that may sign up without an invite. |
| `MAIDAN_SIGNUP_OPEN` | unset | `1` lets any verified email sign up without an invite. Needs `MAIDAN_SIGNUP=1`; refused beside `MAIDAN_SIGNUP_EMAIL_DOMAINS`; in production it also needs `MAIDAN_TURNSTILE_SECRET`. |
| `MAIDAN_SIGNUP_PRIVACY_URL`, `MAIDAN_SIGNUP_TERMS_URL` | unset | The privacy policy and terms, as a path on this origin (`/privacy`) or an `https://` URL, linked from the waitlist page. Any other shape refuses boot. |
| `MAIDAN_TURNSTILE_SECRET` | unset | A Cloudflare Turnstile secret. Set, the waitlist form and the sign-up start need a Turnstile token, checked against Cloudflare (`challenges.cloudflare.com` must be reachable). A missing or rejected token is a 403; an unreachable check is a 502. |
| `MAIDAN_TURNSTILE_SITE_KEY` | with the secret | The public widget key, published in `/.well-known/maidan.json`. Boot refuses one without the other. |
| `MAIDAN_TURNSTILE_HOSTNAME` | host of `MAIDAN_PUBLIC_ORIGIN` | The host name a token must have been solved on. |
| `MAIDAN_SIGNUP_RATE_LIMIT_MAX`, `MAIDAN_SIGNUP_RATE_LIMIT_WINDOW_SECS` | `30` per `3600` s | Per-address limit on the waitlist form, the sign-up start and the operator's invite mint. |
| `MAIDAN_WAITLIST_DOMAIN_RATE_LIMIT_MAX`, `MAIDAN_WAITLIST_DOMAIN_RATE_LIMIT_WINDOW_SECS` | `100` per `3600` s | Per-email-domain limit on waitlist joins. |
| `MAIDAN_AGENT_ADD_RATE_LIMIT_MAX`, `MAIDAN_AGENT_ADD_RATE_LIMIT_WINDOW_SECS` | `20` per `3600` s | Per-workspace limit on adding an agent from the console (`POST /ui/api/workspaces/{wid}/agents`). A refused add is not charged. |
| `MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_MAX`, `MAIDAN_SIGNUP_DOMAIN_RATE_LIMIT_WINDOW_SECS` | `100` per `3600` s | Per-email-domain limit on invite mints and on finished sign-ups, in separate buckets. |

## Demo sandbox

For a throwaway demo instance such as try.maidan.world only
([Deploy](Deploy.md#trymaidanworld)). Off unless `MAIDAN_DEMO_SANDBOX=1`.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_DEMO_SANDBOX` | unset | `1` turns on the public `POST /sandbox`, which gives anyone who asks a fresh workspace with a `#general` channel, a "Say hello" task, an admin token and an agent token, both expiring in 24 hours. Off, the route answers 404. Needs `MAIDAN_PUBLIC_ORIGIN`; refused beside `AUTH_DISABLED` and `MAIDAN_SIGNUP`. With `MAIDAN_TURNSTILE_SECRET` set, the call needs a Turnstile token for the `sandbox` action. |
| `MAIDAN_DEMO_INSTANCE` | unset | `1` says the whole instance is a disposable demo. Under `MAIDAN_ENV=production`, boot refuses `MAIDAN_DEMO_SANDBOX` without it. |
| `MAIDAN_DEMO_SANDBOX_MAX` | `200` | The most workspaces the instance holds. Past it, the mint answers 503 until the next reset. |
| `MAIDAN_DEMO_SANDBOX_RATE_LIMIT_MAX`, `MAIDAN_DEMO_SANDBOX_RATE_LIMIT_WINDOW_SECS` | `3` per `3600` s | Per-address limit on `POST /sandbox`, on whenever the mint is. `0` turns it off. |

## OAuth for MCP clients

See [OAuth](OAuth.md).

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_OAUTH_CLIENTS` | unset | A JSON array of pre-registered clients, inserted at boot. Existing client ids are skipped. Invalid JSON or a bad redirect URI refuses boot. |
| `MAIDAN_OAUTH_CLIENT_METADATA_HOSTS` | unset | Comma-separated exact host names (such as `claude.ai`) that OAuth client metadata documents may be fetched from. Unset, no document is fetched and only pre-registered clients work. A redirecting document is refused. An entry with a scheme, port, path or wildcard refuses boot. |
| `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_MAX`, `MAIDAN_OAUTH_CLIENT_RATE_LIMIT_WINDOW_SECS` | `120` per `60` s | Per client id and address pair, on the token, revocation, authorize and consent endpoints. |
| `MAIDAN_OAUTH_IP_RATE_LIMIT_MAX`, `MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS` | `600` per `60` s | Per address across all client ids, on the same endpoints. |
| `MAIDAN_OAUTH_DYNAMIC_REGISTRATION` | unset (off) | `1` serves RFC 7591 dynamic client registration at `POST /oauth/register` and advertises it as `registration_endpoint`. Public clients only, with loopback, app-scheme or listed `https` redirects. Off, the endpoint answers 404 and clients that registered themselves are refused. Any value but `1`, `true`, `0`, `false` or empty refuses boot. |
| `MAIDAN_OAUTH_DCR_REDIRECT_HOSTS` | unset | Comma-separated exact host names an `https` redirect URI of a self-registered client may name, such as `vscode.dev,insiders.vscode.dev` for VS Code's web redirect. An `https` redirect on any other host is left out of the registration. An entry with a scheme, port, path or wildcard refuses boot. |
| `MAIDAN_OAUTH_REGISTER_RATE_LIMIT_MAX`, `MAIDAN_OAUTH_REGISTER_RATE_LIMIT_WINDOW_SECS` | `20` per `3600` s | Per-address limit on dynamic client registration. Beside it, at most 10,000 registrations may wait on a first sign-in at once. |

## Database

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_DB_MAX_CONNECTIONS` | Postgres `16`, SQLite `1` | Pool size per process. SQLite stays at one connection: concurrent read-modify-write transactions deadlock on the writer upgrade, which a busy timeout cannot resolve. |
| `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS` | `30` | How long a request waits for a pooled connection before it fails instead of hanging. |
| `MAIDAN_DB_STATEMENT_TIMEOUT_MS` | `30000` | Postgres `statement_timeout`, so one runaway query cannot hold a connection. `0` turns it off. Boot migrations are exempt. It also caps the in-server reindex; use the `maidan reindex-embeddings` CLI for a large one. |
| `MAIDAN_DB_LOCK_TIMEOUT_MS` | `10000` | Postgres `lock_timeout`, so a request queued behind a lock fails fast. Boot migrations are exempt. `0` turns it off. |
| `MAIDAN_DB_IDLE_IN_TRANSACTION_TIMEOUT_MS` | `60000` | Postgres `idle_in_transaction_session_timeout`. An idle open transaction holds locks and a snapshot that blocks vacuum; this ends it. `0` turns it off. |
| `MAIDAN_DB_BUSY_TIMEOUT_MS` | `5000` | SQLite `busy_timeout`. |
| `MAIDAN_DB_REPLICA_URL` | unset | A Postgres hot standby to serve reads from. See [Read replicas](Production.md#read-replicas). |

## Artifact storage and uploads

| Variable | Default | Notes |
|---|---|---|
| `ARTIFACT_BACKEND` | `localfs` | `localfs` or `s3`. Use `s3` with more than one replica. |
| `ARTIFACT_LOCALFS_ROOT` | `./.local/artifacts` | The directory for `localfs`. |
| `S3_ENDPOINT`, `S3_BUCKET` | required with `s3` | Any S3-compatible store: AWS S3, Cloudflare R2, MinIO. Requests are path-style. [Providers](Providers.md) has recipes. |
| `S3_REGION` | `us-east-1` | `auto` for R2. |
| `S3_ACCESS_KEY_ID`, `S3_SECRET_ACCESS_KEY` | required with `s3` | A static key; an instance role alone is not enough. |
| `MAIDAN_ARTIFACT_MAX_BODY_BYTES` | `104857600` (100 MiB) | The largest artifact upload body, on `POST /artifacts` and each multipart part. Past it, `413`. A single-shot upload is held in memory while it is hashed; for larger files use the multipart routes on `s3`. |
| `MAIDAN_ARTIFACT_UPLOAD_BUDGET_BYTES` | `268435456` (256 MiB) | The most upload bytes held in memory at once, across all uploads. Each is charged its `Content-Length`, or the full max body when it declares none. An upload that does not fit gets `503` with `Retry-After: 1`. `0` turns it off. Metric `maidan_artifact_upload_budget_refused_total`. |
| `MAIDAN_MEDIA_QUOTA_BYTES` | `21474836480` (20 GiB) | Media each workspace may hold. `0` or `off` lifts it. Any other value that is not a positive whole number keeps the default. See [Media retention and quota](Production.md#media-retention-and-quota). |

## Request limits and proxies

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_RATE_LIMIT_MAX` | `1200` | Requests per window per verified bearer token. A request with no bearer, or one that does not resolve, is keyed by client address. `0` turns it off. `/health*` and `/metrics` are exempt. |
| `MAIDAN_RATE_LIMIT_WINDOW_SECS` | `60` | Read only with an explicit `MAIDAN_RATE_LIMIT_MAX`. |
| `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` | `6000` | Requests per window for one workspace across all its tokens, on `/workspaces/{wid}/…` routes, charged only once the caller is authenticated into that workspace. `0` turns it off. See [Tenant fairness](Production.md#tenant-fairness). |
| `MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS` | `60` | Read only with an explicit `MAIDAN_WORKSPACE_RATE_LIMIT_MAX`. |
| `MAIDAN_RATE_LIMIT_REDIS_URL` | unset | Redis for the rate-limit counters, shared across replicas. Unset or unreachable, counters are per process. |
| `MAIDAN_TRUSTED_PROXY_HOPS` | `0` | How many rightmost `X-Forwarded-For` hops to trust when finding the client address. `0` ignores the header. A shorter or malformed chain falls back to the socket peer. |
| `MAIDAN_MAX_BODY_BYTES` | `2097152` (2 MiB) | The largest request body outside artifact uploads. Past it, `413`. |
| `MAIDAN_MAX_CONCURRENT_REQUESTS` | `1024` | HTTP requests in flight at once. Past it a request gets `503` with `Retry-After: 1` before authentication or rate limiting, so overload costs no database work. A long-poll MCP wait holds its slot; streaming bodies and WebSockets do not. `/health*` and `/metrics` are exempt. `0` turns it off. |
| `MAIDAN_MAX_WS_CONNECTIONS` | `10000` | Concurrent `/ws/subscribe` connections. Past it, an upgrade gets `503`. Client frames are capped at 64 KiB. |
| `MAIDAN_MAX_STREAMS_PER_CREDENTIAL` | `64` | Server-sent event streams one credential may hold open at once: `/mcp/stream`, `/mcp/notifications`, `/agui/stream`, `GET /mcp/streamable` and a `2024-11-05` session's `POST /mcp/streamable`, and A2A `SubscribeToTask`. A credential is the API token (bearer, or a session made from one), or the member when no token is behind the request. Past it, a new stream gets `429` (gRPC `RESOURCE_EXHAUSTED`) before it subscribes to anything. Gauge `maidan_sse_streams`, counter `maidan_sse_streams_refused_total`. `0` turns it off. |

## Live delivery

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_OUTBOX_RELAY` | `1` | `0` turns off the transactional outbox relay (publish in-process after append). Refused in production. |
| `MAIDAN_OUTBOX_RELAY_MODE` | `notify` | `notify` publishes through Postgres `NOTIFY` to every replica. `polled` publishes to this process only; a single-process stopgap when `LISTEN` is broken. |
| `MAIDAN_OUTBOX_POLL_INTERVAL_MS` | `50` | The relay's fast poll, used while it drains and right after activity. |
| `MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS` | `1000` | The idle ceiling. When caught up the relay doubles its sleep up to this; a new row wakes it at once. |
| `MAIDAN_OUTBOX_MAX_ATTEMPTS` | `16` | Failed publishes before a row is quarantined. |
| `MAIDAN_DELIVERY_STABILITY_SECS` | `2` | At-least-once subscriptions deliver only events older than this. Keep it above your longest write transaction. `0` turns the gate off. |
| `MAIDAN_DELIVERY_RECONCILE_MS` | `1000` | The at-least-once reconcile poll. A `NOTIFY` also wakes it. |
| `MAIDAN_BUS_BROADCAST_CAP` | `1024` | The in-process broadcast buffer per channel. Larger lets a slow subscriber fall further behind before it lags, at the cost of memory. |
| `MAIDAN_PRESENCE_HEARTBEAT_SECS` | `10` | How often each replica re-announces its connected members. Cross-replica presence needs Postgres `NOTIFY` mode. |
| `MAIDAN_PRESENCE_TTL_SECS` | `30` | A remote member with no heartbeat for this long leaves the roster. Keep it a small multiple of the heartbeat. |
| `MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS` | `3600` | How long a stateless MCP client's resource subscriptions outlive its last open listener. |

## Work queue

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_CLAIM_DEFAULT_LEASE_SECS` | `600` | The lease a `claim_next_thread` claim gets when the caller names none. 1 s to 7 days; an invalid value keeps the default and is logged. |
| `MAIDAN_CLAIM_REAP_TICK_SECS` | `5` | Seconds between claim-reaper ticks. Each frees up to 1000 claims whose lease lapsed, charges their worked time to the thread's `max_wall_secs` budget and emits `ClaimExpired` (or `ClaimFailed` and a dead-letter entry when that leaves the thread over budget). Every replica runs it; `SKIP LOCKED` splits the work. The four timers below run on this tick. `0` turns it off. Counter `maidan_claims_reaped_total`. |
| `MAIDAN_CLAIM_ACK_TIMEOUT_SECS` | `120` | A leased claim unacknowledged this long gets one `ClaimUnacknowledged`, and the owner and the holder's followers are notified. The claim is not touched. `0` turns it off. |
| `MAIDAN_STALL_SECS` | `900` | A holder showing no progress this long (acknowledgement, own message, own status; a renewal is not progress) makes the thread `stalled` and gets one `ClaimStalled` (`no_progress`). A declared wait is not a stall. `0` turns it off. |
| `MAIDAN_CLAIM_LAPSE_WARN_SECS` | `60` | This long before a lease lapses, one `ClaimStalled` (`lease_lapsing`). Keep it shorter than your leases. `0` turns it off. |
| `MAIDAN_HUMAN_TIMEOUT_SECS` | `3600` | How long an agent may wait on a person (its own `needs_input` or `needs_review` on an acknowledged claim) before the thread returns to the queue with `WaitTimedOut`. The board shows `overdue` from half of it. Nothing is decided for the person. `0` turns it off. |
| `MAIDAN_CLAIM_REAPER_BOOT_GRACE_SECS` | `600` | For this long after the server starts, no lease counts as lapsed: the reaper frees none and `claim_next_thread` takes none over. A restart keeps the server down longer than a short lease, and its holders missed their heartbeats only because it was down; one that comes back within the grace renews its claim and delivers its result. A lease nobody renews is reaped when the grace ends. Each replica holds from its own start. `0` turns it off. |
| `MAIDAN_CHANGE_SWEEP_SECS` | `120` | Seconds between reads of open agent pull requests on GitHub (up to 25 per workspace per sweep). A merge closes the change's thread with `ThreadLanded` and gives each live claim whose footprint covers a merged path a `rebase_needed` notice. A close without a merge leaves the thread open with a notice. Needs the GitHub credential. `0` turns it off. |
| `MAIDAN_CHANGE_CI_BACKSTOP_SECS` | `1800` | With `MAIDAN_GITHUB_WEBHOOK_SECRET` set, a sweep reads a pull request's CI through the Actions API (what a fine-grained token, which cannot read the GraphQL check rollup, falls back to) only when a `workflow_run`, `check_suite`, `check_run` or `status` event arrived since its last read, its head moved, or that read is older than this many seconds. Without the secret every sweep reads it. `0` reads it on every sweep. |
| `MAIDAN_CI_VERDICT_CONTEXTS` | unset | Comma-separated commit status contexts that carry a reviewer's or a gate's verdict rather than CI (for example `review-lane,stamp,security`). `pre-review` is always one. They stay in the gate's read of a pull request, and out of its CI record, so a failing verdict is never reported as `ci_failed` and never wakes a coordinator as a CI failure. |
| `MAIDAN_STALE_BASE_COMMITS` | `50` | Commits behind its branch at which a delivered change round counts as built on a stale base, and gets one `stale_base` notice. |

## Background workers

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_SCHEDULER_TICK_SECS` | unset | Turns on the scheduled-task worker at this interval. Unset, it never starts. |
| `MAIDAN_WAIT_SWEEP_TICK_SECS` | unset | Turns on the wait-escalation sweeper. |
| `MAIDAN_DIGEST_TICK_SECS` | unset | Turns on the email digest sweeper. |
| `MAIDAN_MAIL_WORKER_TICK_SECS` | `5` | The mail worker's tick. It runs whenever SMTP is configured. |
| `MAIDAN_EGRESS_WORKER_TICK_SECS` | `5` | The result-delivery worker's tick (Slack, GitHub, Linear). |
| `MAIDAN_EGRESS_CREDENTIAL_GUARD` | on | `off` stops refusing deliveries whose text looks like it holds a credential ([Result Delivery](Result%20Delivery.md#credential-guard)). Development only: boot refuses `off` with `MAIDAN_ENV=production`. |
| `MAIDAN_WEBHOOK_MAX_ATTEMPTS` | `16` | Failed attempts before an event webhook delivery is quarantined. |
| `MAIDAN_WEBHOOK_POLL_INTERVAL_MS` | `50` | The webhook worker's poll. |
| `MAIDAN_AUTOMATION_MAX_ATTEMPTS` | `16` | Failed attempts before an automation (slash command or FSM hook) delivery is quarantined. |
| `MAIDAN_AUTOMATION_POLL_INTERVAL_MS` | `50` | The automation worker's poll. |
| `MAIDAN_CHAIN_VERIFY_SECS` | unset | Verify every workspace's event-log hash chain at this interval, such as `86400`. See [Event-log integrity](Production.md#event-log-integrity). |
| `MAIDAN_USAGE_METER_FLUSH_SECS` | `60` | How often usage counters are written. |
| `MAIDAN_FREE_ALLOWANCE` | unset | Per-meter free allowance, such as `requests=100000,mcp_calls=50000,stored_bytes=1073741824`. An unknown or repeated meter or a bad number refuses boot. |
| `MAIDAN_OVERAGE_PRICES` | unset | What overage past the free allowance costs, per meter, as `meter=micros/units`: `mcp_calls=1000000/100000` charges 1,000,000 USD micros (one dollar) per 100,000 calls. Comma-separated; both numbers whole and at least 1. `stored_bytes` takes no price (it stays a hard cap). Charged only with `MAIDAN_CREDITS_ADDRESS` set. An unknown or repeated meter or a malformed price refuses boot. See [Credits](Credits.md). |
| `MAIDAN_CREDITS_ADDRESS` | unset | The EVM address that receives USDC top-ups. Unset, credits are off: nothing is charged, no top-up opens and nothing watches the chain. The server only reads the chain; this is an address, never a key. A malformed address refuses boot. |
| `MAIDAN_CREDITS_CHAIN` | `base` | `base` (chain id 8453) or `base-sepolia` (84532, for testing). |
| `MAIDAN_CREDITS_RPC_URL` | the chain's public endpoint | A JSON-RPC endpoint for the chain (`https://mainnet.base.org`, `https://sepolia.base.org` by default). The public endpoints rate-limit `eth_getLogs`; a provider's endpoint is steadier. |
| `MAIDAN_CREDITS_CONFIRMATIONS` | `5` | Blocks a transfer must have on top of it before it is credited. |
| `MAIDAN_CREDITS_POLL_SECS` | `15` | How often the deposit watcher polls the chain. |
| `MAIDAN_OPS_ALERT_CHANNEL` | unset | A Slack channel id (`C…`) that the server posts to on boot and when a background loop panics, stalls, keeps failing or exits. Needs the Slack bot token. See [alerts](https://github.com/david-engelmann/maidan/blob/main/docs/alerts/README.md). |
| `MAIDAN_OPS_FAULT` | unset | `<loop>:<panic\|stall\|fail\|exit>` injects one fault after boot, to prove the alerts. Refused in production. |

## Outbound network

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_PRIVATE_EGRESS_HOSTS` | unset | Comma-separated host names or addresses of peers on a private network (a federation peer, an A2A push receiver, a webhook) that may resolve to private or loopback addresses. Exact matches only. A listed host that resolves to a link-local address is still refused, so cloud metadata stays closed. Works in production. |
| `MAIDAN_INTERNAL_EGRESS_HOSTS` | unset | Comma-separated hostnames under `.internal`, such as `hooks.internal`, that a webhook or other operator-supplied target may name over plain `http://`. Exact matches only; an entry not under `.internal` is ignored, and every unlisted `.internal` name is refused. The platform answers a listed name, so its address is not checked. On Cloudflare the Worker routes each name to a service binding: see [Internal hosts](Hosted%20Operations.md#internal-hosts). |
| `MAIDAN_ALLOW_PRIVATE_EGRESS` | unset | Development only: allow every private and loopback target. Refused in production. |
| `MAIDAN_SECRET_EGRESS_ALLOWLIST` | unset | The instance ceiling on hosts that may receive substituted `secret://` values. Set, a host outside it never receives a value and a workspace cannot add it. Unset, workspace lists alone decide. Set to the empty string, no host receives a value. See [Secret substitution](Production.md#secret-substitution-on-egress). |

Every operator-supplied target (webhook, automation, federation peer, A2A
push, web push) is checked when it is registered and again at each delivery.
A delivery resolves the name, refuses the whole answer if any address is
private, link-local, loopback or reserved, pins the request to those
addresses and never follows redirects. Two lists are the exceptions: a host in
`MAIDAN_PRIVATE_EGRESS_HOSTS` may resolve to a private or loopback address but
never a link-local one, and the answer for a host in
`MAIDAN_INTERNAL_EGRESS_HOSTS` is not checked at all.

## GitHub, Slack and Linear

[Result delivery to GitHub and Slack](Production.md#result-delivery-to-github-and-slack)
explains the permissions and the guards.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_GITHUB_TOKEN` | unset | A fine-grained personal access token for result delivery and the change flow. |
| `MAIDAN_GITHUB_APP_ID` | unset | A GitHub App's id or client id (`Iv23…`). With the two below, every GitHub call authenticates as the app instead of the token. Setting some of the three but not all refuses boot. |
| `MAIDAN_GITHUB_APP_INSTALLATION_ID` | unset | The installation to get tokens for. Must be a positive integer. |
| `MAIDAN_GITHUB_APP_PRIVATE_KEY` | unset | The app's PEM key (`RSA PRIVATE KEY` or PKCS#8). `\n` escapes are accepted. A key that does not parse or is under 2048 bits refuses boot. Prefer the `_FILE` form. |
| `MAIDAN_GITHUB_WRITE_REPOS` | unset | Comma-separated `owner/name`s the GitHub credential may write to, compared case-insensitively. Unset, every GitHub write is refused and boot warns. A workspace's egress allowlist can narrow this, never widen it. |
| `MAIDAN_GITHUB_WEBHOOK_SECRET` | unset | Turns on the inbound GitHub webhook route, `POST /integrations/github/events`. Its CI events (`workflow_run`, `check_suite`, `check_run`, `status`) then decide when the change sweep reads a pull request's CI (`MAIDAN_CHANGE_CI_BACKSTOP_SECS`). |
| `MAIDAN_MARK_READY_APP_ID` | unset | The UUID of the one installed app whose installations may call `POST /operator/github/mark-ready` and `/operator/github/enqueue`. Unset, every call is refused. |
| `MAIDAN_MARK_READY_BASES` | unset | Comma-separated `owner/name=branch` pairs: the only base each listed repository's agent pull requests may be marked ready into. An unlisted repository is decided by the workspace allowlist alone. A malformed entry, an invalid branch, `prod` or a repeated repository refuses boot. |
| `MAIDAN_MERGE_QUEUE_LABELS` | unset | Comma-separated `owner/name=label` pairs: the label that enters a ready agent pull request into that repository's merge queue. Only the mark-ready app may add or remove it. Leave out any repository a person merges. |
| `MAIDAN_SLACK_BOT_TOKEN` | unset | The Slack bot token for result delivery and operator alerts. |
| `MAIDAN_SLACK_SIGNING_SECRET` | unset | Turns on the inbound Slack routes. |
| `MAIDAN_LINEAR_API_KEY` | unset | A Linear API key or OAuth token, for results delivered to Linear issues ([Result Delivery](Result%20Delivery.md#linear)). Unset, a Linear delivery fails each attempt. |
| `MAIDAN_DEPLOY_HOOK_SECRET` | unset | Turns on `POST /integrations/deploys/events`, signed like a GitHub webhook. Unset, the route is a 404. See [Swarms](Swarms.md). |

## Notifications

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_SMTP_HOST`, `MAIDAN_SMTP_FROM` | unset | Both set turn on notification mail. |
| `MAIDAN_SMTP_PORT` | `587` | |
| `MAIDAN_SMTP_USERNAME`, `MAIDAN_SMTP_PASSWORD` | unset | |
| `MAIDAN_SMTP_STARTTLS` | on | `false` or `0` for a plaintext relay. |
| `MAIDAN_EMAIL_PRESENCE_WINDOW_SECS` | unset | Skip a notification email when the recipient was seen within this many seconds. Unset, every opted-in recipient is mailed. |
| `MAIDAN_VAPID_PRIVATE_KEY`, `MAIDAN_VAPID_PUBLIC_KEY`, `MAIDAN_VAPID_SUBJECT` | unset | Web Push. All three turn it on: a base64url P-256 private scalar, the uncompressed public key and a `mailto:` or `https:` contact. Unset, there is no web push and no key is generated. |
| `MAIDAN_WEBPUSH_LIVE_WINDOW_SECS` | `60` | A member seen within this many seconds counts as connected and is not pushed. |
| `MAIDAN_WEBPUSH_WORKER_TICK_SECS` | `5` | How often failed pushes are retried. |

## Embeddings and search

[Embeddings](Embeddings.md) covers providers and tuning.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_EMBEDDING_PROVIDER` | `hash-v1` | `hash-v1` (offline, lower quality) or `openai-compatible`. |
| `MAIDAN_EMBEDDING_ENDPOINT` | with `openai-compatible` | The full embeddings URL, such as `http://localhost:1234/v1/embeddings` for LM Studio. A chat completion endpoint does not work. |
| `MAIDAN_EMBEDDING_MODEL` | with `openai-compatible` | The model id sent in each request. |
| `MAIDAN_EMBEDDING_API_KEY` | unset | Bearer token for the provider. |
| `MAIDAN_EMBEDDING_DIM` | probed | The embedding dimension. Unset, the server embeds one probe string at boot and uses its length. |
| `MAIDAN_EMBEDDING_TIMEOUT_SECS` | `15` | Provider request timeout. |
| `INDEXER_STALE_SECS` | `0` | Above `0`, readiness degrades when the indexer has an unhandled message event older than this and has made no progress for as long. An idle instance stays ready. Recommended `300` on Postgres with embeddings. |
| `MAIDAN_INDEXER_QUEUE_CAPACITY` | `1024` | Messages queued for embedding; a full queue applies backpressure. |
| `MAIDAN_INDEXER_BATCH_SIZE` | `32` | Messages per provider call. |
| `MAIDAN_INDEXER_RETRIES` | `3` | Retries after a failed call (at most 10). |
| `MAIDAN_INDEXER_RETRY_BASE_MS` | `250` | First retry delay, doubling up to 10 s. |
| `MAIDAN_EMBED_REPAIR_INTERVAL_SECS` | `300` | How often the repair sweep embeds messages that were missed. `0` turns it off. |
| `MAIDAN_EMBED_REPAIR_BATCH` | `256` | Messages per repair pass. |
| `MAIDAN_HNSW_M`, `MAIDAN_HNSW_EF_CONSTRUCTION`, `MAIDAN_HNSW_EF_SEARCH` | pgvector's | The pgvector HNSW index parameters. See [Query tuning](https://github.com/david-engelmann/maidan/blob/main/docs/Query-Tuning.md). |

## Federation

| Variable | Default | Notes |
|---|---|---|
| `FEDERATION_DISABLED` | unset | `1` stops the outbound poll worker. Inbound federation keeps serving; revoke a peer to stop it. |
| `FEDERATION_POLL_INTERVAL_SECS` | `30` | Each tick polls every enabled peer. A peer that is refused, has no secret or does not answer is logged and skipped. Counter `maidan_federation_peer_polls_total{outcome}`. |

## A2A

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_A2A_GRPC_ADDR` | unset | Bind address for the A2A gRPC server, such as `0.0.0.0:50051`. Unset, gRPC is off. An address that does not parse refuses boot. |
| `MAIDAN_A2A_GRPC_PLAINTEXT` | unset | `1` acknowledges that TLS is terminated in front of the gRPC listener. Required when the bind address is not loopback. |
| `MAIDAN_A2A_PUBLIC_ORIGIN` | unset | Makes the Agent Card's HTTP interface URLs absolute, such as `https://maidan.example.com`. Unset, they are host-relative. |
| `MAIDAN_A2A_GRPC_PUBLIC_ADDR` | unset | The reachable gRPC `host:port` to advertise. Set it to add a gRPC interface to the Agent Card. |

## Retention

All ages are in days. [Retention](Production.md#retention) says what each one
prunes and what it never touches.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_RETENTION_EVENTS_DAYS` | off | Event-log rows. |
| `MAIDAN_RETENTION_AUDIT_DAYS` | off | Audit rows, including instance-level ones. |
| `MAIDAN_RETENTION_NOTIFICATIONS_DAYS` | off | Read notifications. |
| `MAIDAN_RETENTION_DELIVERIES_DAYS` | off | Finished delivery rows. |
| `MAIDAN_RETENTION_MESSAGES_DAYS` | off | Messages, erased as a purge erases them. |
| `MAIDAN_RETENTION_MEDIA_DAYS` | `30` | Unreferenced media. `0` or `off` keeps media until someone erases it. |
| `MAIDAN_RETENTION_SWEEP_SECS` | `86400` | How often the sweeper runs. |
| `MAIDAN_RETENTION_BATCH` | `5000` | Rows deleted per batch. |

A value that is not a positive whole number turns that knob off. For
`MAIDAN_RETENTION_MEDIA_DAYS` that means media is kept.

## Observability

| Variable | Default | Notes |
|---|---|---|
| `OTLP_ENDPOINT` | unset | A gRPC OTLP collector for traces, and for metrics when `OTLP_METRICS=1`. Request spans carry the method, the path (never the query) and the headers, with credentials and signatures printed as `Sensitive`. |
| `OTLP_SERVICE_NAME` | `maidan-server` | The `service.name` resource attribute. |
| `OTLP_METRICS` | unset | `1` also pushes metrics over OTLP, beside the `/metrics` scrape. |
| `OTLP_METRICS_ENDPOINT` | `OTLP_ENDPOINT` | A separate collector for metrics. |
| `OTLP_METRICS_INTERVAL_SECS` | `15` | Metrics push interval. |

OTLP export is the `otel` Cargo feature, on by default. A build without it
(`--no-default-features`) drops the OpenTelemetry stack; logs and `/metrics`
still work, and an `OTLP_ENDPOINT` that is set is reported at startup and
ignored.

## Experimental: Jev land-gate advice

Off by default, and no part of the land-gate decision itself. Turning it on
adds only `POST /threads/:id/land-gate/advice` (`thread:transition`), which
takes a bounded description of the work as `state` (a string, array or object)
and optional `thresholds` (`green_min_confidence`, `red_min_confidence`). It
answers with the model's raw verdict, probabilities and confidence, plus a
`recommended_land` in which a green or red below its threshold becomes amber;
the policy never promotes an answer to green. The route never writes or arms
the gate: a verifier with the `land_gate` skill still records the pass or fail.
A disabled advisor is `404`, and a provider failure is `502` on the advice call
only. `scripts/eval-land-gate-advisor.py` scores recorded answers against
independently labelled outcomes. The supplied `state` leaves the deployment
for TypeSafe when the route is called, so send no secrets or unapproved data.

| Variable | Default | Notes |
|---|---|---|
| `MAIDAN_JEV_LAND_GATE_ENABLED` | `0` | `1` turns on the advisory route. An invalid value refuses boot. With it off, none of the others are read. |
| `TYPESAFE_API_KEY` | required when on | Never sent to clients or logged. |
| `MAIDAN_JEV_MODEL` | `jev-latest` | A model or alias from TypeSafe `GET /v1/models`. |
| `MAIDAN_JEV_GREEN_MIN_CONFIDENCE` | `0.90` | Green below this becomes amber. In `[0,1]`. |
| `MAIDAN_JEV_RED_MIN_CONFIDENCE` | `0.90` | Red below this becomes amber. In `[0,1]`. |
| `MAIDAN_JEV_TIMEOUT_MS` | `1500` | Per-request timeout. A timeout fails only the advice call, with `502`. |
| `MAIDAN_JEV_BASE_URL` | `https://api.typesafe.ai/` | An origin only, behind the same egress guard as other outbound calls. |

When it is on, a missing key, a malformed threshold, an unsafe base URL or a
failed DNS lookup refuses boot.

## Build options

These are chosen when the image is built, not at run time.

| Option | Effect |
|---|---|
| `bootstrap` Cargo feature | The unauthenticated seed routes `MAIDAN_BOOTSTRAP` opens. On for `cargo run`; off in every published image. `MAIDAN_ENABLE_BOOTSTRAP=1` is the image build argument that turns it on. |
| `otel` Cargo feature | OTLP export. On by default. |
| `sqlite-vec` Cargo feature | SQL `vec_distance_cosine` for semantic search on SQLite. Off by default; SQLite ranks in process without it. |
| `mcp-frame-capture` Cargo feature | Logs full MCP frames with `MAIDAN_LOG=info,maidan_mcp::frame=debug`, with secrets redacted. A release build that enables it fails to compile. |
