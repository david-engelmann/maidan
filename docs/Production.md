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
| `MAIDAN_ENV`    | no       | Set to `production` to forbid `AUTH_DISABLED` outright.       |
| `AUTH_DISABLED` | no       | Serve every request unauthenticated. **Fail-closed:** takes effect only when `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` is *also* set, and never when `MAIDAN_ENV=production` (either violation refuses boot). A stray `AUTH_DISABLED=1` alone now fails startup loudly instead of silently serving an open workspace. Dev/test/CI only. |
| `MAIDAN_ALLOW_INSECURE_NO_AUTH` | no | Explicit acknowledgement required to honor `AUTH_DISABLED`. Never set in production. |
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
| `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` | no | When **> 0**, per-workspace fairness limit (`v110.0.0`): caps total requests for one workspace across **all** its tokens, on `/workspaces/{wid}/…` routes (incl. search). Default off. Independent of the global limit; reuses the Redis backend when set. |
| `MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS` | no | Per-workspace fixed window in seconds (default `60`). |
| `MAIDAN_PRESENCE_HEARTBEAT_SECS` | no | Interval at which each replica re-announces its locally-connected members over `maidan_presence` (default `10`). Cross-replica presence is active only in Postgres NOTIFY mode. |
| `MAIDAN_PRESENCE_TTL_SECS` | no | A remote member with no heartbeat for this long is dropped from the merged roster (default `30`). Keep it a small multiple of the heartbeat. |
| `MAIDAN_DB_MAX_CONNECTIONS` | no | Pool size per process. Default preserves the dialect default (**Postgres 16**, **SQLite 8**). See the replica caveat below. |
| `MAIDAN_DB_ACQUIRE_TIMEOUT_SECS` | no | How long a request waits for a free pooled connection before erroring instead of hanging (default `30`). Under saturation this surfaces a clean `500`/timeout rather than blocking indefinitely. |
| `MAIDAN_DB_STATEMENT_TIMEOUT_MS` | no | Postgres per-connection `statement_timeout`. **Default `30000` (30 s)** — caps runaway queries so one can't pin a pooled connection indefinitely. Set `0` to disable. See the caveat below. |
| `MAIDAN_DB_BUSY_TIMEOUT_MS` | no | SQLite `busy_timeout` (default `5000`). |
| `MAIDAN_DELIVERY_STABILITY_SECS` | no | At-least-once delivery (`v125.0.0`) stability window: a subscribe with `at_least_once` only delivers events whose insert time is older than this. Must exceed the longest insert-transaction duration. Default `2`; `0` disables the gate. |
| `MAIDAN_DELIVERY_RECONCILE_MS` | no | Poll cadence for the at-least-once reconcile loop (a NOTIFY also wakes it). Default `1000`. |

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
others. It is **independent** of the per-client `MAIDAN_RATE_LIMIT_MAX`: enable
either or both. With `MAIDAN_RATE_LIMIT_REDIS_URL` set, the per-workspace counter
is shared across replicas; otherwise it is per-process. Start generous (a
legitimate large workspace shouldn't hit it in normal use) and tighten only if a
noisy tenant is observed. Not a substitute for hard CPU/IO isolation — that is
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
DATABASE_URL=postgres://… maidan init --workspace my-team --admin-handle david
```

It runs migrations, creates the initial workspace and an admin member, mints an
all-capabilities bearer token, and prints that token **once** (to stdout; save it).
It **refuses if the database already has a workspace**, so it can never clobber an
existing deployment or mint a second root token. Use the printed token to mint
narrower per-agent tokens via the API. The production image can stay
bootstrap-stripped (`--no-default-features`), since `init` writes through the store
rather than the bootstrap HTTP routes.

### HTTP bootstrap (alternative)

When bearer auth is enabled, unauthenticated `POST /workspaces` and
`POST /workspaces/:wid/members` require `MAIDAN_BOOTSTRAP=1` **and** an image built
with the `bootstrap` Cargo feature. Only the **first** workspace may be created via
bootstrap; a second `POST /workspaces` returns `403`. Typical seed (private network):

1. Set `MAIDAN_BOOTSTRAP=1`, `AUTH_DISABLED=1`, and `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` (the acknowledgement — `AUTH_DISABLED` alone now refuses to boot).
2. Create workspace + member, mint admin token.
3. Unset those flags, set `MAIDAN_ENV=production`, restart.

Integration tests use `AUTH_DISABLED=1` + `MAIDAN_ALLOW_INSECURE_NO_AUTH=1` (bootstrap flag not required).

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
| Object store (`ARTIFACT_BACKEND=s3`) | Artifacts written on one replica must be readable on another. Do **not** use `localfs` with multiple replicas. |
| `MAIDAN_SESSION_SECRET` | Must be **identical** on every replica so subscribe-resume tokens (and session signing) validate regardless of which replica issued them. |

**Still pod-local (do not assume cross-replica):**

- In-flight **MCP streamable sessions** and open WebSocket/SSE subscriptions live on the replica that holds the connection; a reconnect may land elsewhere and resumes from the durable cursor, not in-memory buffer.
- A **running reindex job** executes on the replica that started it; only its *status* is durable and queryable from any replica. If that replica dies mid-run the row stays `Running` — re-issue the (idempotent) reindex.

**Rolling updates / boot:** every replica runs migrations on boot, serialized by
a Postgres advisory lock (`v105.0.0`) so concurrent starts against a fresh or
upgrading database don't race on DDL. Because pre-`v1.0.0` migrations are not
guaranteed backward-compatible with the previous binary, prefer
`maxUnavailable: 0` (surge) rolling updates, or run migrations as a pre-deploy
step; from `v1.0.0` the API is stable but treat schema changes as
expand-then-contract. `/health/ready` gates traffic on DB + object store +
indexer + the `LISTEN` bus, so an LB honoring readiness won't route to a replica
mid-migration.

**Not covered:** load/throughput benchmarking (bench harness, Cluster 109),
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

## Backup & disaster recovery (`v260.0.0`)

Maidan's durable state is two things, and the backup story follows the same split:

| What | Store | Backed up by |
|------|-------|--------------|
| System of record — every workspace, member, channel, thread, message, event log, audit trail, token, follow/pref/schedule | **Postgres** (`DATABASE_URL`) | `pg_dump -Fc` |
| Content-addressed artifact blobs (immutable, deduped) | `localfs` root **or** an object store (`ARTIFACT_BACKEND=s3`) | a tar of the localfs root; for S3 the bucket itself is the durable copy |

Two operator scripts implement it:

- **`scripts/backup.sh [BACKUP_DIR]`** — `pg_dump` (custom format) plus, for
  `localfs`, a `tar` of `ARTIFACT_LOCALFS_ROOT`; writes a `MANIFEST.txt`. For
  `s3`, the bucket is the durable copy — enable **bucket versioning** and/or
  cross-region replication there rather than copying blobs into the backup.
- **`scripts/restore.sh <backup-dir> [--force]`** — `pg_restore` into the target
  `DATABASE_URL` (+ untar artifacts). It **refuses a non-empty target** unless
  `--force`, so a restore can't silently clobber a live database; `--force` restores
  with `--clean --if-exists`.

**Not in the data backup — restore these from your secret manager, out of band:**
`DATABASE_URL`, `MAIDAN_SESSION_SECRET` (subscribe-resume/session signing),
`FEDERATION_ENCRYPTION_KEY` (+ any `FEDERATION_DECRYPT_KEYS` — see the Cluster-189
rotation keyring), and SMTP/OIDC credentials. A DB dump without the session secret
still restores all data; only signed-token continuity needs the same secret.

**RPO / RTO.** A periodic `backup.sh` (e.g. hourly cron) gives an RPO of one backup
interval. For a tighter RPO, run Postgres with **WAL archiving / PITR** (or a managed
Postgres with continuous backup) — the logical dump is the portable floor, not the
lower bound. RTO is a `restore.sh` run plus a `/health/ready` check before the load
balancer is pointed at the restored instance.

**Recovery outline.** Provision Postgres + the artifact store → set the out-of-band
secrets → `DATABASE_URL=… ARTIFACT_LOCALFS_ROOT=… scripts/restore.sh <dir> --force`
→ start one replica and confirm `/health/ready` is `200` (it gates on DB + object
store + indexer + the `LISTEN` bus) → scale out. Because artifacts are
content-addressed, a message referencing a blob that predates the artifact backup is
still consistent after restore; a blob written *after* the last artifact archive is
the only thing a stale artifact backup can miss.

## API stability

From `v1.0.0`, HTTP and MCP shapes are semver-stable. Pre-1.0 releases
may break without migration shims.
