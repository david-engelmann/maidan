# Production deployment

Guidance for running Maidan at `v1.0.0` and later. Security overview:
[Threat-Model.md](Threat-Model.md).

## Probes

| Endpoint          | Use        | Behavior                                      |
|-------------------|------------|-----------------------------------------------|
| `GET /health/live`  | Liveness   | Always `200` if the process is up.            |
| `GET /health/ready` | Readiness  | `200` when DB, artifact store, indexer (if stale check enabled and no embedding errors), and Postgres `LISTEN` bus (when used) are healthy. |
| `GET /health`       | Readiness  | Alias of `/health/ready`.                     |

## Environment

| Variable        | Required | Notes                                                |
|-----------------|----------|------------------------------------------------------|
| `DATABASE_URL`  | yes      | Postgres (recommended) or SQLite.                    |
|                 |          | SQLite connections enable `foreign_keys`, WAL, and `busy_timeout=5000` ms automatically. |
| `MAIDAN_ENV`    | no       | Set to `production` to forbid `AUTH_DISABLED`.       |
| `AUTH_DISABLED` | no       | Must **not** be set in production.                   |
| `MAIDAN_BOOTSTRAP` | no    | Set to `1` only during initial seed when auth is on **and** the server was built with the `bootstrap` Cargo feature (default for local dev; **off** in the production Docker image unless `MAIDAN_ENABLE_BOOTSTRAP=1` at image build). Allows unauthenticated `POST /workspaces` and `POST /workspaces/:wid/members`. Only the **first** workspace may be created via bootstrap; remove the flag and restart after minting tokens. |
| `FEDERATION_ENCRYPTION_KEY` | when federation is used | 32-byte secret (base64 or hex) used to encrypt peer outbound bearer tokens at rest. Required to create peers and for the poll worker after restart. Back up with your DB; rotation requires re-creating peers. |
| `FEDERATION_DISABLED` | no | Set to `1` to disable the outbound poll worker. |
| `FEDERATION_POLL_INTERVAL_SECS` | no | Outbound poll interval (default `30`). |
| `MAIDAN_EMBEDDING_PROVIDER` | no | `hash-v1` (default) or `openai-compatible`. |
| `MAIDAN_EMBEDDING_ENDPOINT` | when provider is `openai-compatible` | Full URL to embeddings endpoint (OpenAI-compatible response shape). |
| `MAIDAN_EMBEDDING_MODEL` | when provider is `openai-compatible` | Embedding model id sent in request body. |
| `MAIDAN_EMBEDDING_API_KEY` | optional | Bearer token for remote provider. |
| `MAIDAN_EMBEDDING_DIM` | no | Expected embedding dimension (default `1024`). |
| `MAIDAN_EMBEDDING_TIMEOUT_SECS` | no | HTTP timeout for remote embeddings (default `15`). |
| `INDEXER_STALE_SECS` | no | When **> 0**, `/health/ready` is degraded if the embedding indexer has not observed an event for this many seconds. Default `0` (disabled). **Recommended `300`** on Postgres deployments with embeddings enabled. |
| `GET /metrics` | no | Prometheus text exposition (HTTP + subscribe recovery + indexer/bus gauges). Label cardinality is fixed (no workspace UUIDs). |
| `OTLP_ENDPOINT` | no | gRPC OTLP collector URL for **traces** (and metrics when `OTLP_METRICS=1`). |
| `OTLP_SERVICE_NAME` | no | Resource `service.name` for OTLP (default `maidan-server`). |
| `OTLP_METRICS` | no | Set to `1` to push the same `metrics` crate instruments to OTLP (fanout with Prometheus scrape). Requires `OTLP_ENDPOINT` unless `OTLP_METRICS_ENDPOINT` is set. |
| `OTLP_METRICS_ENDPOINT` | no | Override OTLP gRPC URL for metrics only. |
| `OTLP_METRICS_INTERVAL_SECS` | no | Periodic push interval (default `15`). |
| `MAIDAN_RATE_LIMIT_MAX` | no | When **> 0**, global HTTP rate limit per bearer token (or `X-Forwarded-For` / `anonymous`). Default off. `/health/*` and `/metrics` exempt. |
| `MAIDAN_RATE_LIMIT_WINDOW_SECS` | no | Fixed window length in seconds (default `60`). |
| `MAIDAN_RATE_LIMIT_REDIS_URL` | no | When set, global and per-token quotas use Redis fixed-window counters (multi-replica). Falls back to in-memory if unset or connection fails. |
| `MAIDAN_PRESENCE_HEARTBEAT_SECS` | no | Interval at which each replica re-announces its locally-connected members over `maidan_presence` (default `10`). Cross-replica presence is active only in Postgres NOTIFY mode. |
| `MAIDAN_PRESENCE_TTL_SECS` | no | A remote member with no heartbeat for this long is dropped from the merged roster (default `30`). Keep it a small multiple of the heartbeat. |

### Local embedding servers (e.g. LM Studio)

Maidan's indexer uses the **OpenAI-compatible embeddings** API shape, not chat
completion. Point `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` at your server's
**embeddings** URL (for example `http://localhost:1234/v1/embeddings`) and set
`MAIDAN_EMBEDDING_MODEL` to the loaded model id. A chat endpoint such as
`http://localhost:1235/api/v1/chat` is not used for search indexing.

## Bootstrap

When bearer auth is enabled, unauthenticated `POST /workspaces` and
`POST /workspaces/:wid/members` require `MAIDAN_BOOTSTRAP=1`. Only the **first**
workspace may be created via bootstrap; a second `POST /workspaces` returns
`403`.

Typical production seed (private network):

1. Set `MAIDAN_BOOTSTRAP=1` and `AUTH_DISABLED=1`.
2. Create workspace + member, mint admin token.
3. Unset both flags, set `MAIDAN_ENV=production`, restart.

Integration tests use `AUTH_DISABLED=1` (bootstrap flag not required).

Human browser login via OIDC ships in **`v2.0.0`**. See [OIDC.md](OIDC.md) for design
detail. Summary:

| Variable | Required | Notes |
|----------|----------|-------|
| `MAIDAN_OIDC_ENABLED` | when using OIDC | `1` enables `/auth/oidc/*` and session routes. |
| `MAIDAN_SESSION_SECRET` | when OIDC on | HMAC key for signed `maidan_session` cookies (32+ bytes). Bare session UUIDs in cookies are rejected. |
| `MAIDAN_OIDC_ISSUER` | yes (non-mock) | IdP issuer URL for discovery. |
| `MAIDAN_OIDC_CLIENT_ID` | yes (non-mock) | OAuth client id. |
| `MAIDAN_OIDC_CLIENT_SECRET` | confidential clients | Code exchange secret. |
| `MAIDAN_OIDC_REDIRECT_URI` | yes | Registered callback (e.g. `https://host/auth/oidc/callback`). |
| `MAIDAN_OIDC_MOCK` | no | `1` for deterministic dev/CI only; forbidden when `MAIDAN_ENV=production`. |
| `MAIDAN_OIDC_FIRST_ADMIN` | no | Default on: session may mint the first `token:admin` per workspace via `POST /auth/session/mint`. Set `0` to disable. |
| `MAIDAN_COOKIE_SECURE` | no | Set `1` in production for `Secure` session cookies. |
| `MAIDAN_OIDC_POST_LOGOUT_REDIRECT_URI` | no | Registered post-logout redirect (e.g. `https://host/ui/`). Used when IdP exposes `end_session_endpoint`. |
| `MAIDAN_OIDC_AUTO_MINT` | no | `1` redirects to `/ui/?auto_mint=1` after login when the workspace has no `token:admin` yet; the UI then calls `POST /auth/session/mint`. Off by default. Requires first-admin mint (`MAIDAN_OIDC_FIRST_ADMIN` not `0`). |
| `MAIDAN_SESSION_SECRET` | when auth on (or OIDC) | HMAC key for signed `resume_token` and session cookies (32+ bytes). |
| `MAIDAN_SUBSCRIBE_RESUME_SECRET` | no | Override HMAC key for subscribe resume tokens only. |
| `MAIDAN_SUBSCRIBE_RESUME_TTL_SECS` | no | Resume token lifetime in seconds (default `3600`). |

After OIDC login, use `/ui/` (session cookie) or mint an API token for MCP.

**Channel browser (`v92.0.0`):** From `/ui/`, list channels and threads, then post
messages via `POST /ui/api/...` using the session cookie — no bearer or curl required.
Bearer tokens still work for the same flows when pasted in the header field.
Remove `MAIDAN_BOOTSTRAP` once the first human has `token:admin`.

## API discovery

| Endpoint            | Use                                      |
|---------------------|------------------------------------------|
| `GET /openapi.json` | Machine-readable OpenAPI 3.0 (Track W.1). HTTP routes and `application/problem+json` errors; subscribe/resume protocol summary in `info.description`. Auth/session routes are under the `auth` tag (`/auth/oidc/*`, `/auth/session`, `/ui/api/...`). |

## WebSocket and MCP subscribe (`v4.0.0`)

Real-time subscribers use **`GET /ws/subscribe`** (WebSocket) or **`GET /mcp/stream`**
(SSE). Both share the same control frames and event envelope shape.

MCP resource subscription notifications use **`GET /mcp/notifications`** (SSE JSON-RPC
lines) with **`POST /mcp`** for `resources/subscribe` / `tools/call` — requires
`workspace:read` (same as resource read). Distinct from `/mcp/stream` workspace events.

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

### Delivery reliability metrics (`v6.0.0`)

Scrape `GET /metrics` and alert on subscribe recovery paths (labels are fixed —
no per-workspace series).

| Metric | Symptom | Suggested action |
|--------|---------|------------------|
| `maidan_bus_lag_total` rising | In-process subscribers falling behind the broadcast buffer | Check publish rate; scale consumers; ensure clients use `workspace_id` filter for auto-replay |
| `maidan_subscribe_replay_total{outcome="replay_hint"}` | Lag without workspace scope or auto-replay failed | Fix client filter; inspect store/DB errors in logs |
| `maidan_subscribe_replay_total{outcome="replay_truncated"}` sustained | Event log replay hitting 500-row window | Client should loop on `after_id` / `resume_token` until truncation stops |
| `maidan_indexer_last_event_age_seconds` high (with `INDEXER_STALE_SECS` set) | Indexer silent while messages post | Check embedding provider errors on `/health`; verify indexer task running |
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
| `MAIDAN_OUTBOX_POLL_INTERVAL_MS` | `50` | Outbox relay poll interval. |
| `MAIDAN_OUTBOX_RELAY` | `1` (enabled) | Set `0` to disable relay (append-then-publish in-process). **`MAIDAN_ENV=production` rejects `MAIDAN_OUTBOX_RELAY=0`.** |

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

### Agent observability (`v76.0.0`)

Scrape `GET /metrics` for agent-substrate health (see [[Agent Integration]]). Gate e2e: `agent_substrate_gate_e2e.rs`.

| Metric / signal | Symptom | Suggested action |
|-----------------|---------|------------------|
| `maidan_bus_lag_total` | Subscribers behind | Scope WS filters; scale consumers |
| `maidan_indexer_last_event_age_seconds` | Stale embeddings | Fix embedding provider; run `maidan reindex-embeddings` |
| `maidan_outbox_pending` / quarantined | Relay stuck | [[Production#Outbox relay]] |
| `maidan_automation_delivery_total{outcome="failure"}` | Slash/FSM HTTP failing | [[Production#Automation HTTP delivery]] |
| MCP tool latency | Not exported per-tool yet | Use HTTP request metrics + logs |

Example Grafana dashboard (Prometheus datasource): `docs/dashboards/maidan-operator.json` (`v89.0.0`).

SLO alert templates (Prometheus / Alertmanager): `docs/alerts/` (`v90.0.0`). Validate with `./scripts/validate-prometheus-rules.sh`.

**Semantic scale:** set `MAIDAN_EMBEDDING_PROVIDER=openai-compatible` in Helm prod values; run `maidan reindex-embeddings --database-url $DATABASE_URL` after provider changes.

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
omit `workspace_id` for all workspaces (`token:admin`). CLI `maidan reindex-embeddings`
remains for shell/CI. Jobs are in-process (not durable across restarts).

| `GET /workspaces/:wid/search` | See table above. OpenAPI `SearchHit` documents `embedding_model`. |
| `GET /metrics`    | Prometheus text (HTTP counters, subscribe replay, indexer age, bus listener). |
| `DELETE /messages/:id/purge` | Hard-delete a **tombstoned** message (GDPR erasure); requires bearer with `workspace:write`. |
| `POST /workspaces/:id/purge` | Deep workspace erasure (`v28.0.0`): tombstone+purge all messages, remove embeddings/references, revoke API tokens, delete event log; returns counts JSON. |
| `GET /workspaces/:id/audit` | Workspace-scoped audit trail (`workspace:read`). |

Import into Swagger UI, Redoc, or your client generator. The document
version tracks the server release (`info.version`).

## Helm (production)

Charts under `helm/maidan` (server) and `helm/maidan-stack` (optional Postgres + MinIO).

| Values file | Use |
|-------------|-----|
| `values.yaml` | Dev defaults |
| `values-prod.yaml` | HPA + ingress (manual TLS secret) |
| `values-cert-manager.yaml` | Ingress + `cert-manager.io/cluster-issuer` annotation |
| `values-profile-otel.yaml` | JSON logs + OTLP traces/metrics (`OTLP_ENDPOINT`, `OTLP_METRICS=1`) |
| `values-profile-redis.yaml` | `MAIDAN_RATE_LIMIT_REDIS_URL` (multi-replica quotas) |
| `values-profile-s3.yaml` | S3-compatible `ARTIFACT_BACKEND` |
| `values-ci.yaml` | kind smoke (SQLite, auth off) |

Layer profiles as needed; see `helm/maidan/PROFILES.md` for example `helm upgrade` commands (`v88.0.0`).

**cert-manager:** install [cert-manager](https://cert-manager.io/) and a `ClusterIssuer`, then:

```bash
helm install maidan ./helm/maidan -f ./helm/maidan/values-cert-manager.yaml -n maidan --create-namespace
```

**CI validation:** `./scripts/helm-template-smoke.sh` and `./scripts/helm-install-kind-smoke.sh` (kind + Docker).

Set `secrets.DATABASE_URL` in values (not a `MAIDAN_` prefix). For the umbrella chart, substitute `RELEASE-postgresql` / `RELEASE-minio` hostnames in `maidan-stack/values-prod.yaml` with your Helm release name.

## API stability

From `v1.0.0`, HTTP and MCP shapes are semver-stable. Pre-1.0 releases
may break without migration shims.
