# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [90.0.0] — 2026-06-03

### Added

- SLO alert templates: `docs/alerts/prometheus-rules-maidan-slo.yaml`, Alertmanager route example, validation script.
- Contract test tying alert rules to exported `/metrics` names.

## [89.0.0] — 2026-06-03

### Added

- OTLP metrics push (`OTLP_METRICS`, `OTLP_METRICS_ENDPOINT`) with Prometheus scrape fanout.
- Example Grafana dashboard `docs/dashboards/maidan-operator.json`.
- OpenTelemetry SDK bumped to 0.31 for traces and metrics.

## [88.0.0] — 2026-06-03

### Added

- Helm production profile overlays (OTel, Redis rate limits, S3) and `helm/maidan/PROFILES.md`.
- Helm template smoke coverage for profile combinations.

## [87.0.0] — 2026-06-03

### Added

- Operator reindex job API: `POST /operator/reindex-embeddings`, `GET /operator/reindex-embeddings/:job_id`.
- `Search::reindex_embeddings` for Postgres and SQLite backends.

### Fixed

- SQLite workspace-scoped `maidan reindex-embeddings` / job reindex UUID filter binding.

## [86.0.0] — 2026-06-03

### Added

- Optional `embedding_model` query param on semantic HTTP search and MCP `search_messages`.

## [85.0.0] — 2026-06-02

### Changed

- `sqlite-vec` is an optional Cargo feature on `maidan-search` (default off).
- CI job verifies linkage with `--features sqlite-vec`; SQLite semantic search without the feature uses in-process cosine ranking.

## [84.0.0] — 2026-06-02

### Added

- `MAIDAN_OUTBOX_RELAY_MODE` (`notify` | `polled`) and `MAIDAN_OUTBOX_POLL_INTERVAL_MS`.
- Production guard: `MAIDAN_ENV=production` rejects `MAIDAN_OUTBOX_RELAY=0`.
- SQLite deployments enable outbox relay by default; NOTIFY-loss runbook in [[Production]].

## [83.0.0] — 2026-06-02

### Added

- Product Ladder closure for SQLite `maidan_delivery_cursor` parity (store impl since `v56.0.0`).
- `delivery_cursor` integration tests for Postgres and in-memory SQLite watermarks.

## [82.0.0] — 2026-06-02

### Added

- Context export pagination: `message_cursor` / `thread_cursor` on HTTP and MCP tools.
- `Store::list_messages_after` with stable message ordering (`posted_at`, `id`).

## [81.0.0] — 2026-06-02

### Added

- WS/MCP subscribe `channel_grants` for private channel access control.
- DM subscribe auto-grants the backing private DM channel.

## [80.0.0] — 2026-06-02

### Added

- Unified operator delivery API at `/workspaces/:wid/deliveries` (webhook + automation via `kind`).
- Webhook delivery list/get/replay in store (per workspace).

## [79.0.0] — 2026-06-02

### Added

- A2A `tasks/cancel` RPC and `SubscribeToTask` `statusUpdate` progress frames for non-terminal tasks.
- Terminal subscribe error `-32005`; cancel/progress e2e in `a2a_protocol_e2e`.

## [77.0.0] — 2026-06-02

### Added

- `contracts/http-capability-map.json` and OpenAPI parity CI.
- `http_capability_matrix_e2e` table-driven HTTP capability denial.
- OpenAPI documentation for automation, apps, DMs, workspace context, multipart.

## [76.0.0] — 2026-06-01

### Added

- Agent observability runbook and `agent_substrate_gate_e2e` (`maidan-agent-1.0` gate).

## [75.0.0] — 2026-06-01

### Changed

- Production guidance for real embedding providers and `maidan reindex-embeddings`.

## [74.0.0] — 2026-06-01

### Added

- MCP tools `get_thread_context` and `get_workspace_context`.

## [73.0.0] — 2026-06-01

### Added

- MCP streamable session close e2e; documented session lifecycle in [[Agent Integration]].

## [72.0.0] — 2026-06-01

### Added

- Persisted A2A push config and tasks; `SubscribeToTask` / `tasks/resubscribe` SSE.
- Best-effort HTTP push on task updates.

## [71.0.0] — 2026-06-01

### Added

- `contracts/ws-subscribe-filter.schema.json`; EventKind forward-compat docs.
- MCP resource-notification parity script in CI.

## [70.0.0] — 2026-06-01

### Changed

- [[Architecture]], [[Remaining Work]], [[Open Work]], and root `README.md` reflect **`v69.0.0`** agent substrate (no stale “pins absent” / pre–2.0 stubs).

## [69.0.0] — 2026-06-01

### Added

- `contracts/mcp-capability-map.json` and `contracts/http-capability-routes.json`.
- Table-driven MCP capability matrix e2e (deny + allow gate per tool).
- HTTP capability contract denials in `capability_matrix_e2e`.
- CI: `mcp_capability_map_contract` and `http_capability_map_contract` in `check-agent-contract.sh`.

## [68.0.0] — 2026-06-01

### Added

- Durable signed HTTP delivery queue for slash commands and FSM hooks (`maidan_automation_deliveries`).
- `AutomationDeliveryWorker` with retries, quarantine, and Prometheus metrics.
- Operator API: `GET /workspaces/:wid/automation/deliveries`, `GET .../automation/dlq`, `GET .../deliveries/:did`, `POST .../deliveries/:did/replay`.
- Env: `MAIDAN_AUTOMATION_MAX_ATTEMPTS`, `MAIDAN_AUTOMATION_POLL_INTERVAL_MS`.

## [67.0.0] — 2026-06-01

### Added

- `GET /workspaces/:id/context` packs channels and thread contexts (with message edit history).
- Thread context responses include `message_edits`.

## [66.0.0] — 2026-06-01

### Added

- `/.well-known/maidan.json` documents MCP endpoints and agent card URL.

## [65.0.0] — 2026-06-01

### Added

- App OAuth: `POST .../apps/:app_id/oauth/authorize` and `POST /oauth/app/token` exchange.

## [64.0.0] — 2026-06-01

### Added

- Per-token capability quotas enforced on MCP `tools/call`.

## [63.0.0] — 2026-06-01

### Added

- MCP capability denial covered in `agent_surfaces_e2e`.

## [62.0.0] — 2026-06-01

### Added

- WebSocket `subscribe_ack` includes `schema_version: 1`.
- `GET /workspaces/:wid/outbox/quarantined` lists poison outbox rows.

## [61.0.0] — 2026-06-01

### Added

- `GET /.well-known/agent-card.json` for A2A discovery.
- A2A `tasks/pushNotificationConfig/set` and `/get` for workspace webhooks.

## [60.0.0] — 2026-06-01

### Added

- MCP streamable session TTL (`MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS`, default 3600s).
- `DELETE /mcp/streamable` closes a session (`Mcp-Session-Id` header).

## [59.0.0] — 2026-06-01

### Added

- [[Agent Integration]] guide for external agents.
- Contract golden files: `contracts/event-kinds.json`, `contracts/mcp-tool-names.json`.
- `scripts/check-agent-contract.sh` in CI.

## Maidan 2.0 product gate — 2026-06-01

Tag **[`maidan-2.0`](https://github.com/david-engelmann/maidan/releases/tag/maidan-2.0)**
marks Product Ladder **35–58** completion at the same commit as **`v58.0.0`**.
Checklist: [`docs/Product Completion Checklist.md`](docs/Product%20Completion%20Checklist.md).

Semver **`v2.0.0`** remains **Cluster 2.0** (OIDC identities and human sessions).

## [58.0.0] — 2026-06-01

### Added

- Maidan 2.0 product completion checklist refresh (Clusters 28–57 critical path).
- Expanded `product_completion_gate_e2e`: OpenAPI, metrics, apps, webhooks, app-installations.

## [57.0.0] — 2026-05-31

### Added

- Workspace installed apps: `maidan_apps`, `maidan_app_installations`, bot `MemberKind::Agent` per install.
- App tokens via `api_tokens.app_installation_id`; capabilities must be a subset of the installation grant.
- HTTP: register/list apps, install, list/revoke installations, `POST .../app-installations/:iid/tokens`.

## [56.0.0] — 2026-05-31

### Added

- SQLite `maidan_delivery_cursor` (migration 0023) with real `get` / `advance` store methods.
- `POST /workspaces/:wid/outbox/:outbox_id/replay` clears quarantine for operator recovery (`workspace:write`).

## [55.0.0] — 2026-05-28

### Added

- Helm production bundle: `ingress.annotations`, `values-cert-manager.yaml`, `maidan-stack/values-prod.yaml`.
- `values-ci.yaml` and `scripts/helm-install-kind-smoke.sh` with CI job `helm install (kind)`.
- Helm secrets use `DATABASE_URL` (matches server config).

## [54.0.0] — 2026-05-28

### Added

- Per-token capability quotas: `maidan_token_quotas` and `quotas` on API token mint.
- Quota middleware enforces limits per capability after bearer auth (429 + `Retry-After`).
- Optional Redis rate limiter via `MAIDAN_RATE_LIMIT_REDIS_URL` (global + per-token keys).
- `AuthContext.token_id` for bearer-authenticated requests.

## [53.0.0] — 2026-05-28

### Added

- Workspace full erasure: `DELETE /workspaces/:id` with `confirm_workspace_id` body.
- `Store::erase_workspace` runs deep purge then deletes the workspace row (CASCADE-owned data).

## [52.0.0] — 2026-05-28

### Added

- FSM automation hooks: register handlers for `ThreadStateChanged` transitions (optional `from_state` / `to_state` filters).
- `POST/GET/DELETE /workspaces/:wid/fsm-hooks` with `http` or `mcp_tool` handlers and HMAC signing for HTTP.
- `FsmHookWorker` dispatches on the event bus (covers HTTP transitions and federation-ingested state changes).
- MCP tools `register_fsm_hook` and `list_fsm_hooks`.
- `maidan_fsm_hooks` migrations (Postgres v23, SQLite v21).

## [51.0.0] — 2026-05-29

### Added

- Slash commands: `/name args` parsed on `post_message` when a workspace command is registered.
- `POST/GET/DELETE /workspaces/:wid/slash-commands` with `http` or `mcp_tool` handlers.
- MCP tools `register_slash_command` and `list_slash_commands`.
- Handler results stored on the posted message under `metadata.slash_command` / `metadata.slash_response`.

## [50.0.0] — 2026-05-28

### Added

- Outbound webhooks: subscribe to `EventKind` filters per workspace.
- `POST/GET/DELETE /workspaces/:wid/webhooks` with HMAC-SHA256 signed delivery and retry queue.
- `maidan_webhook_subscriptions` and `maidan_webhook_deliveries` migrations (Postgres v21, SQLite v19).

## [49.0.0] — 2026-05-28

### Added

- `GET /threads/:id/context` — messages, references, metadata-linked artifacts, FSM history.
- `Store::list_thread_transitions` for thread lifecycle audit in context export.

## [48.0.0] — 2026-05-29

### Added

- `sqlite-vec` loaded per SQLite connection; SQL-side `vec_distance_cosine` for semantic search.
- `SearchHit.score` in `[0, 1]` — comparable across Postgres and SQLite within one search mode.

## [47.0.0] — 2026-05-29

### Added

- Per-model embedding tables (`maidan_embedding_models`, `maidan_emb_*`) for mixed dimensions.
- `maidan reindex-embeddings` CLI to rebuild vectors after provider change.

## [46.0.0] — 2026-05-29

### Added

- `maidan_message_edits` stores body before/after on each edit.
- `GET /messages/:id/edits` and `GET /ui/api/messages/:mid/edits`.
- UI v5: “edited” labels and edit history panel in the collab view.

## [45.0.0] — 2026-05-29

### Added

- UI v4 admin tab: workspace audit log, purge confirmation, federation peer admin.
- Token mint with capabilities and revoke by ID in `/ui`.
- `GET /ui/api/workspaces/:wid/audit` and `GET /ui/api/workspaces/:wid/peers`.

## [44.0.0] — 2026-05-29

### Added

- UI v3 collaboration at `/ui`: thread list, post/edit messages, artifact upload, faceted search.
- Session/bearer read proxies: `GET /ui/api/channels/:cid/threads`,
  `GET /ui/api/threads/:tid/messages`, `GET /ui/api/workspaces/:wid/search`.

## [43.0.0] — 2026-05-29

### Added

- UI v2 at `/ui`: responsive shell, workspace channel list, WebSocket live event tail.
- `GET /ui/api/workspaces/:wid/channels` for browser session or bearer.

## [42.0.0] — 2026-05-29

### Added

- WebSocket ephemeral presence (`presence_snapshot`, online/away/offline) and typing
  indicators when subscribe includes `member_id` and `filter.workspace_id`.

## [41.0.0] — 2026-05-29

### Added

- Emoji reactions: `maidan_reactions`, message reaction HTTP API, MCP tools, and bus events.
- Thread pins: `maidan_pins`, pin/unpin/list HTTP API, MCP tools, and bus events.

## [40.0.0] — 2026-05-29

### Added

- Member inbox: `maidan_inbox_cursor`, `GET /members/:id/inbox`, `POST /members/:id/inbox/read`.
- Baseline `@handle` mention routing in `maidan-router` when messages are posted (HTTP + MCP).

## [39.0.0] — 2026-05-29

### Added

- Direct messages: `maidan_dm_conversations` schema, HTTP `POST/GET /workspaces/:id/dm`,
  `POST/GET /dm/:id/messages`, MCP `open_dm_conversation` / `list_dm_conversations` /
  `post_dm_message`, and WebSocket `filter.dm_conversation_id` (resolves to thread).

## [38.0.0] — 2026-05-29

### Added

- MCP `notifications/resources/updated` fan-out on HTTP `edit_message`, `purge_workspace`,
  `create_mention`, and `cast_vote`.

## [37.0.0] — 2026-05-29

### Added

- A2A `SendStreamingMessage` on `POST /a2a/v1/rpc`: SSE stream of JSON-RPC frames with initial
  `Task` and `TaskStatusUpdateEvent` when a message is posted.

## [36.0.0] — 2026-05-29

### Added

- `maidan mcp-stdio` supports Postgres `DATABASE_URL` (`PostgresStore` + `PostgresSearch`).

## [35.0.0] — 2026-05-29

### Added

- MCP streamable HTTP bidirectional mux: follow-up `POST /mcp/streamable` requests with an open
  `Mcp-Session-Id` return JSON-RPC responses pushed to the original SSE session.

## [34.0.0] — 2026-05-29

### Added

- `Mcp-Session-Id` response header on `POST /mcp/streamable` for streamable HTTP session correlation.

## [33.0.0] — 2026-05-29

### Added

- MCP `notifications/resources/updated` fan-out when HTTP tombstones a message or transitions thread FSM state.

## [32.0.0] — 2026-05-29

### Added

- `helm/maidan-stack` umbrella chart with optional Bitnami PostgreSQL and MinIO dependencies.
- Helm template smoke covers maidan-stack when `Chart.lock` is present.

## [31.0.0] — 2026-05-28

### Added

- Workspace deep purge removes artifact metadata for workspace members and deletes content-addressed blobs from the artifact store.
- `WorkspacePurgeResult.artifacts_removed`; audit metadata `artifact_blobs_deleted`.

## [30.0.0] — 2026-05-28

### Added

- Optional HTTP rate limiting via `MAIDAN_RATE_LIMIT_MAX` and `MAIDAN_RATE_LIMIT_WINDOW_SECS`.
- `429 Too Many Requests` with `application/problem+json` and `Retry-After`.

## [29.0.0] — 2026-05-28

### Added

- `PATCH /messages/:id` — edit message body/metadata; sets `edited_at`; publishes `MessageEdited`.
- MCP `edit_message` tool with author vs moderator capability rules.
- Search indexer and embedding handler react to `MessageEdited`.

## [28.0.0] — 2026-05-28

### Added

- Deep workspace purge: embeddings, references, API token revocation, event log removal; extended `WorkspacePurgeResult` counts.
- `GET /workspaces/:id/audit` — workspace-scoped audit trail for operators.

### Changed

- `POST /workspaces/:id/purge` audit metadata includes full purge counts.

## [27.0.0] — 2026-05-28

Major release: **Product Ladder 17–27 close** (clusters 23–27 shipped in PR #198;
CHANGELOG sections v23–v26 record logical cluster boundaries at the same merge).

### Added

- MCP streamable HTTP: `POST /mcp/streamable` returns JSON-RPC response then SSE notifications on one connection.
- Post-ladder backlog: `docs/Remaining Work.md` and vault refresh.

### Documentation

- Retros: `docs/Retros/Cluster 23.0.md` … `Cluster 27.0.md`.

## [26.0.0] — 2026-05-28

### Added

- Product completion checklist and `product_completion_gate_e2e` smoke.

## [25.0.0] — 2026-05-28

### Added

- `POST /workspaces/:id/purge` workspace message erasure with `workspace.purge` audit events.

## [24.0.0] — 2026-05-28

### Added

- `helm/maidan` chart (Deployment, Service, ConfigMap, Secret, Ingress, HPA, PVC) and `scripts/helm-template-smoke.sh`.

## [23.0.0] — 2026-05-28

### Added

- Web UI tabs: events, search, thread FSM transitions, member API token mint.

## [22.0.0] — 2026-05-28

### Added

- Capability map documentation and denial e2e tests for HTTP, MCP, A2A, and WS.

## [21.0.0] — 2026-05-28

Major release: Google A2A protocol v1.0 ingress and client.

### Added

- `POST /a2a/v1/rpc` with `SendMessage` and `GetTask`.
- `maidan-a2a::A2aClient` and protocol types.

## [20.0.0] — 2026-05-28

Major release: message router crate wired into HTTP and MCP.

### Added

- `maidan-router` resolve helpers for channel, thread, and message chains.
- SQLite integration test; server and MCP fan-out use the router.

## [19.0.0] — 2026-05-28

Major release: S3 multipart large artifacts.

### Added

- S3 multipart upload API and MinIO integration test.
- HTTP multipart routes and MCP multipart tools.

## [18.0.0] — 2026-05-28

Major release: SQLite semantic search.

### Added

- SQLite `maidan_message_embeddings` migration and semantic `Search` impl.
- HTTP/MCP `mode=semantic` on SQLite backends.

### Changed

- Cosine ranking in Rust (sqlite-vec SQL deferred; see Decisions).

## [17.0.0] — 2026-05-28

Major release: MCP resource fan-out for tool mutations.

### Added

- `maidan-mcp::resource_updates` resolves thread, channel, workspace, and artifact URIs from mutating tools.
- Notifications fan out to all subscribed related resources.

### Changed

- MCP reference documents multi-URI fan-out behavior.

## [16.0.0] — 2026-05-28

Major release: MCP HTTP resource notification SSE.

### Added

- Shared `McpServer` on `AppState` for persistent HTTP subscriptions.
- `GET /mcp/notifications` SSE stream of JSON-RPC notifications.
- Broadcast fan-out for `notifications/resources/updated` (HTTP + stdio).

### Changed

- `POST /mcp` uses shared dispatcher; MCP reference documents HTTP notifications.

## [15.0.0] — 2026-05-28

Major release: MCP resource subscriptions (stdio first).

### Added

- MCP JSON-RPC methods `resources/subscribe` and `resources/unsubscribe`.
- Stdio notification delivery: `notifications/resources/updated`.
- Resource URI validation helper in `maidan-mcp`.
- `post_message` trigger mapping to notify subscribed `maidan://threads/{id}` resources.

### Changed

- MCP reference now documents subscription methods and notification shape.

## [14.0.0] — 2026-05-28

Major release: SQLite transactional outbox parity.

### Added

- SQLite `maidan_outbox` migration and transactional `append_event`.
- `OutboxBackend` for Postgres and SQLite; relay + metrics on both dialects.
- SQLite deployments run outbox relay against `InMemoryBus` after commit.

### Changed

- `AppState.outbox_backend` replaces `outbox_pool` for dialect-neutral metrics.

## [13.0.0] — 2026-05-27

Major release: delivery cursors and subscriber idempotency contract.

### Added

- Postgres `maidan_delivery_cursor` (`consumer_id`, `workspace_id` → `last_delivered_log_id`).
- Optional `consumer_id` on WebSocket subscribe and MCP SSE; replay floors from stored cursor.
- Federation ingest advances `federation:{peer_id}` cursor after successful handoff.
- Delivery contract documented in Decisions, Architecture, Production.

## [12.0.0] — 2026-05-27

Major release: outbox relay quarantine and operator metrics.

### Added

- `quarantined_at` on `maidan_outbox`; relay skips quarantined rows.
- `MAIDAN_OUTBOX_MAX_ATTEMPTS` (default 16) caps failed relay retries.
- Metrics `maidan_outbox_quarantined`, `maidan_outbox_oldest_pending_seconds`,
  `maidan_outbox_relay_total{result="quarantined"}`.
- Production runbook for quarantine triage and manual recovery.

## [11.0.0] — 2026-05-27

Major release: coverage depth — outbox/relay tests and CI floor at 11%.

### Added

- Postgres outbox integration tests (`record_attempt`, `mark_published`, ordering).
- `maidan-bus::test_support` bus doubles (`FailingBus`, `RecordingBus`).
- Server tests: `publish` deferral when `outbox_relay`, relay failure path, HTTP outbox e2e,
  `/metrics` outbox gauges, `GET /ui/` static e2e.

### Changed

- `COVERAGE_MIN_LINES` raised from **10.5** to **11.0** in CI.

## [10.0.0] — 2026-05-27

Major release: Postgres transactional outbox for commit-then-publish ordering.

### Added

- `maidan_outbox` table; `append_event` enqueues outbox rows in the same transaction.
- `OutboxRelay` background task publishes pending rows via `PostgresBus`.
- Metrics `maidan_outbox_pending` and `maidan_outbox_relay_total{result}`.
- Integration tests for outbox enqueue and relay delivery.

### Changed

- Postgres `publish()` defers direct `bus.publish` to the relay; SQLite unchanged.
- Federation ingest uses a single `publish()` path (fixes double append).

## [9.0.0] — 2026-05-27

Major release: coverage depth — targeted tests and raised CI line floor.

### Added

- Unit/e2e tests for `EventFilter`, bus hydrate/error paths, subscribe metrics,
  `/metrics` hydrate scrape, search query edges, and auth peer decrypt failure.

### Changed

- `COVERAGE_MIN_LINES` raised from **10.0** to **10.5** in CI.
- WS auto-replay integration test timeout extended for slow CI hosts.

## [8.0.0] — 2026-05-27

Major release: Postgres bus hydrate observability on `/metrics`.

### Added

- `maidan_bus_notify_hydrate_total{result}` (`ok`, `not_found`, `failed`,
  `invalid_payload`) for Postgres NOTIFY pointer hydrations.
- `HydrateStats` in `maidan-bus`; exported via `AppState.bus_hydrate_stats` on scrape.
- Production/Operations/Architecture hydrate alerting and troubleshooting.

### Changed

- OpenAPI `/metrics` description includes hydrate series (Postgres deployments).

## [7.0.0] — 2026-05-27

Major release: Postgres bus pointer delivery — NOTIFY carries `log_id`, listener
hydrates from `maidan_events`.

### Added

- `Store::get_stored_event(log_id)` on Postgres and SQLite.
- Postgres `NOTIFY` pointer payload (`log_id_v1`) with listener hydration;
  `BusError::HydrateNotFound` and `HydrateFailed` for missing or corrupt rows.
- Integration tests for pointer round-trip and large persisted events.

### Changed

- Postgres `publish` with `log_id > 0` no longer ships full envelopes on NOTIFY
  (legacy full JSON retained for `log_id == 0` synthetic publishes).
- [[Architecture]], [[Decisions]], and [[Production]] document pointer-default
  semantics and unchanged at-most-once standing risk.

## [6.0.0] — 2026-05-27

Major release: delivery reliability observability for subscribe recovery and
background task health.

### Added

- Prometheus metrics for subscriber lag/recovery across WebSocket and MCP SSE:
  `maidan_bus_lag_total`, `maidan_bus_lag_skipped`, and
  `maidan_subscribe_replay_total{transport,outcome}`.
- Runtime gauges on `/metrics`: `maidan_indexer_last_event_age_seconds`,
  `maidan_bus_listener_ok`, and `maidan_bus_listener_errors_total`.
- Production/Operations/Architecture guidance for delivery reliability alerts and
  troubleshooting.

### Changed

- Full `compose.yaml` profile now sets `INDEXER_STALE_SECS=300` to surface indexer
  silence in readiness during smoke-style runs.

## [5.0.0] — 2026-05-27

Major release: coverage uplift, optional Codecov, and model-aware semantic search.

### Added

- Targeted unit tests; CI line-coverage floor raised to **10.0%** (`COVERAGE_MIN_LINES`).
- Optional Codecov upload from the `llvm-cov` job when `CODECOV_TOKEN` is configured.
- Postgres `semantic_search` filters embeddings by the active provider `model`.
- `SearchHit.embedding_model` on semantic hits; `/health` reports embedding model and dimension when enabled.
- Architecture and Production documentation for lexical vs semantic `rank` semantics.

### Changed

- `Search::semantic_search` takes an explicit `model` argument (breaking for implementors).
- OpenAPI `SearchHit` schema includes optional `embedding_model`.

## [4.0.0] — 2026-05-27

Major release: subscriber continuity with signed resume tokens and replay truncation signaling.

### Added

- HMAC-signed `resume_token` and `subscribe_ack` on WebSocket subscribe and MCP SSE (`/mcp/stream`).
- `replay_truncated` control frame when event-log replay returns 500 rows (`REPLAY_LIMIT`).
- Production and Architecture documentation for subscribe/resume; OpenAPI `info.description` summary.
- E2e: resume-token reconnect and `replay_truncated` when the log exceeds one replay window.

### Changed

- Full-profile `compose.yaml` sets `MAIDAN_SESSION_SECRET` so auth-on smoke tests start with resume signing configured.

## [3.0.0] — 2026-05-27

Major release: search/subscriber depth with semantic facets, automatic lag replay, and a CI coverage floor.

### Added

- Semantic facets on Postgres search (`author`, `channel`, `kind`) for `mode=semantic` on HTTP and MCP.
- Automatic WS/MCP replay from `maidan_events` when subscribers lag and `workspace_id` scope is present.
- Coverage gate in CI with `cargo llvm-cov --fail-under-lines` (`COVERAGE_MIN_LINES=9.0`).

### Changed

- `replay_hint` is now a fallback path (no workspace filter or replay failure), not the primary lag path when workspace scope exists.
- Operations runbook documents the measured baseline (9.8% lines from run `26485125992`) and gate bump policy.

## [2.1.0] — 2026-05-26

Minor release: OIDC operator hardening after `v2.0.0`.

### Added

- HMAC-signed `maidan_session` cookies; unsigned bare UUID cookies rejected.
- IdP `end_session_endpoint` discovery and redirect on `POST /auth/logout`.
- OpenAPI documentation for auth/session routes and `sessionCookie` security scheme.
- `MAIDAN_OIDC_AUTO_MINT=1` redirects to `/ui/?auto_mint=1` when no `token:admin` exists.
- `/ui/` improvements: session-aware controls, one-time secret banner, copy-to-clipboard.

### Changed

- `MAIDAN_SESSION_SECRET` is load-bearing for cookie integrity (invalidates existing sessions on upgrade).
- OpenAPI document version `2.1.0`.

## [2.0.0] — 2026-05-26

Major release: runtime OIDC human login, server-side sessions, and browser UI
integration. Agent MCP/A2A paths remain bearer-token authenticated.

### Added

- Migration `0012`: `maidan_oidc_identities`, `maidan_sessions`, `maidan_oidc_pending`.
- OIDC routes: `GET /auth/oidc/login`, `GET /auth/oidc/callback`, `POST /auth/logout`.
- Session routes: `GET /auth/session`, `POST /auth/session/mint` (first `token:admin` per workspace).
- `GET /ui/api/workspaces/:wid/events` with session-or-bearer middleware.
- `/ui/` HTML: OIDC sign-in, session status, first-admin token mint, cookie-backed events.
- `MAIDAN_OIDC_*` and `MAIDAN_SESSION_*` configuration (see `docs/Production.md`).
- `Store::workspace_has_active_capability` for admin-mint gating.
- `openidconnect` v4 client with mock IdP for tests (`MAIDAN_OIDC_MOCK=1`).

### Changed

- `docs/OIDC.md` design spike superseded by runtime implementation.
- `deny.toml`: ignore `RUSTSEC-2023-0071` for transitive `rsa` via `openidconnect`.

## [1.4.0] — 2026-05-26

Auth hardening minor: bootstrap route gating and OIDC design planning.

### Added

- `MAIDAN_BOOTSTRAP=1` gate on unauthenticated bootstrap routes when auth is enabled.
- One-shot bootstrap workspace seed behavior (`POST /workspaces` returns 403 after first workspace).
- OIDC human login design spike document (`docs/OIDC.md`) with phased `v2.0.0` plan.

### Changed

- `Store` gained `count_workspaces` for bootstrap enforcement.
- Production and threat-model docs now reflect bootstrap gating and OIDC deferral.

## [1.3.0] — 2026-05-26

Semantic search UX minor: HTTP/MCP semantic mode, remote embedding provider
support, and readiness visibility for embedding/indexer failures.

### Added

- `mode=semantic` for `GET /workspaces/:wid/search` (Postgres semantic ranking).
- MCP `search_messages.mode` (`lexical` / `semantic`) with parity behavior.
- OpenAI-compatible embedding provider via env:
  `MAIDAN_EMBEDDING_PROVIDER=openai-compatible`,
  `MAIDAN_EMBEDDING_ENDPOINT`, `MAIDAN_EMBEDDING_MODEL`,
  optional `MAIDAN_EMBEDDING_API_KEY`, `MAIDAN_EMBEDDING_DIM`,
  `MAIDAN_EMBEDDING_TIMEOUT_SECS`.
- `/health/ready` now reports embedding indexer errors.

### Changed

- Semantic query paths now fail fast on embedding provider errors (HTTP + MCP).
- `EmbeddingProvider::embed` returns `Result<Vec<f32>, EmbeddingProviderError>`.

## [1.2.0] — 2026-05-26

Search + embeddings minor: pluggable provider hook, faceted lexical search,
Postgres web-style query operators.

### Added

- `EmbeddingProvider` trait and `MAIDAN_EMBEDDING_PROVIDER` (default `hash-v1`).
- Optional `author`, `channel`, and `kind` filters on workspace search (HTTP + MCP).
- Postgres `websearch_to_tsquery` when `q` contains quotes, `-negation`, or `or`.

### Changed

- `Search::search_messages` accepts `SearchFilters`; both backends apply facets in SQL.

## [1.1.0] — 2026-05-24

Delivery reliability minor: bus health, client replay, federation secrets + pull smoke.

### Added

- Postgres `LISTEN` task health on `/health/ready` (`bus` field).
- WebSocket and MCP `replay_hint` when the in-process bus subscriber lags.
- `after_id` on `/ws/subscribe` and MCP stream; persisted event replay on connect.
- Migration 0010: ChaCha20-Poly1305 encrypted peer outbound bearer secrets (`FEDERATION_ENCRYPTION_KEY`).
- Migration 0011: `maidan_peers.remote_workspace_id` for cross-instance poll.
- `scripts/federation-pull-smoke.sh` and CI pull-path compose coverage.

### Changed

- Federation poll worker resolves outbound secrets from DB after restart.
- `CreatePeer` accepts optional `remote_workspace_id`.

## [1.0.0] — 2026-05-24

Production gates and semver-stable public API. Deployment guidance in
`docs/Production.md`. Liveness/readiness probes and production config
guards shipped in `v0.7.0`; this release documents the contract and
freezes breaking-change policy.

### Added

- `docs/Production.md` production runbook.
- Documented API stability policy (see `docs/Decisions.md`).

## [0.7.0] — 2026-05-24

End of Cluster H. Web UI, MCP stdio, SSE stream, production ergonomics.

### Added

- Graceful shutdown and `X-Request-Id` middleware.
- `/health/live` and `/health/ready` probes.
- `maidan mcp-stdio` CLI transport.
- `GET /mcp/stream` SSE for subscribed events.
- Minimal browser UI at `/ui/`.
- `docs/Production.md`; `MAIDAN_ENV=production` forbids `AUTH_DISABLED`.

## [0.6.0] — 2026-05-24

End of Cluster G. Maidan-native federation between deployments.

### Added

- Migration 0009 `maidan_peers` and `maidan_federated_ingest` dedupe table.
- `maidan-a2a` federation envelope, batch validation, and `Outbound` HTTP client.
- `POST /a2a/v1/events` inbound ingest with peer bearer auth.
- `FederationWorker` background poll (`FEDERATION_POLL_INTERVAL_SECS`, `FEDERATION_DISABLED`).
- Peer admin API and `GET /.well-known/maidan.json` agent card.
- Capabilities `federation:ingest` and `federation:admin`.

## [0.5.0] — 2026-05-23

End of Cluster F. API tokens, capabilities, and auth on HTTP, WebSocket, and MCP.

### Added

- Migration 0008 `maidan_api_tokens` + store CRUD (create, lookup, revoke).
- `maidan-auth` — token hashing, capability vocabulary, `AuthContext`.
- HTTP Bearer middleware; `AUTH_DISABLED=1` for tests and bootstrap.
- Per-route capability checks with RFC 7807 401/403 responses.
- WebSocket `SubscribeFrame.token` with `event:subscribe` enforcement.
- MCP auth on `tools/call`, `resources/read`, `prompts/get`.
- `POST /workspaces/:wid/members/:mid/tokens` and `DELETE /tokens/:id`.

## [0.4.0] — 2026-05-23

End of Cluster E. Artifacts are first-class: S3-backed object storage,
typed kinds, HTTP upload/download, and MCP tools.

### Added

- `ArtifactKind` (`screenshot`, `recording`, `transcript`, `code_dump`, `attachment`).
- Migration 0007 kind CHECK on both dialects.
- `S3Store` with MinIO testcontainers integration test.
- `POST /artifacts`, `GET /artifacts/:sha`, `GET /artifacts/:sha/meta`.
- `put_reader` streaming helper and kind-aware `put_*` helpers.
- MCP `upload_artifact`, `get_artifact_metadata`, `maidan://artifacts/{sha}`.

### Changed

- Compose `full` profile uses `ARTIFACT_BACKEND=s3` + `minio-init` bucket job.
- Rust toolchain pinned to **1.91** (AWS SDK MSRV).

## [0.3.0] — 2026-05-23

End of Cluster D. Thread lifecycle is FSM-driven with a persistent
transition log, hierarchical nested threads, Postgres embedding
indexing, event replay, and MCP workflow prompts.

### Added

- `maidan-fsm` thread lifecycle (`open` → `in_review` → `closed` → `archived`).
- Schema 0004 `maidan_thread_transitions`; schema 0005 `parent_thread_id`.
- `POST /threads/:id` with `start_review`, `close`, `archive` actions.
- `ThreadStateChanged` on the event bus.
- `maidan_fsm::hsm` parent/child state ordering for nested threads.
- `EmbeddingHandler` with `hash-v1` deterministic 1024-d vectors (Postgres).
- Schema 0006 `maidan_events` persistent log + `GET /workspaces/:wid/events`.
- MCP `prompts/list` and `prompts/get` (`thread_workflow`).

### Changed

- `ThreadState` includes `in_review`.
- Server publishes append to `maidan_events` before bus notify.

## [0.2.0] — 2026-05-23

End of Cluster C. The workspace is now searchable: lexical search on
both backends, vector search on Postgres, and the async indexer
pipeline that future clusters will use for embedding generation.

### Added

- `maidan-search::Search` async trait with `search_messages`,
  `upsert_embedding`, `semantic_search`.
- `PostgresSearch` lexical impl using `tsvector` + GIN index +
  `ts_headline` snippets (migration 0002).
- `SqliteSearch` lexical impl using FTS5 + `snippet()` (migration
  0002). FTS5 grammar-escaped queries.
- `PostgresSearch` semantic impl using `pgvector` `vector(1024)` +
  HNSW cosine index (migration 0003). SQLite returns
  `SearchError::Unsupported` for semantic methods.
- `GET /workspaces/:wid/search?q=...&limit=...` HTTP route with
  RFC 7807 `application/problem+json` errors on bad input.
- MCP `search_messages` tool (8th tool overall) sharing the same
  `Arc<dyn Search>` as the HTTP route.
- `maidan-search::Indexer` task: subscribes to the bus, reconnects
  with exponential backoff, dispatches to a swappable `EventHandler`.
- `LoggingHandler` baseline + `wait_for(timeout, predicate)` test
  helper.
- `maidan-server::main` wires the indexer on boot and shuts it
  down cleanly on serve exit.

### Changed

- Every Postgres testcontainer in the workspace switched from
  `postgres:17-alpine` to `pgvector/pgvector:pg17` so migration
  0003's `CREATE EXTENSION vector` succeeds.
- `AppState::new` signature gained `search: Arc<dyn Search>`.
- `McpServer::new` signature gained the same.

### Security

- FTS5 query strings are escaped before binding to prevent grammar
  injection. (Not a SQL injection concern — values are always
  parameterized — only an FTS5 operator concern.)

## [0.1.0] — 2026-05-23

End of Cluster B. The substrate from `v0.0.1` is now reachable: HTTP
CRUD covers the core entity set, every mutation publishes to the bus,
clients can subscribe over WebSocket, and an MCP surface exposes the
workspace as tools and resources to agents.

### Added

- GitHub Actions CI workflows: `lint` (fmt + clippy + cargo-deny),
  `secrets` (trufflehog), `test` (unit), `integration`
  (testcontainers Postgres + in-memory SQLite), `e2e` (docker compose
  + `/health` smoke). All five required-status-checks on `main`.
- Nightly mutation + benchmark workflow skeleton (informational).
- Release workflow that builds cross-arch binaries (Linux x64/arm64
  + macOS x64/arm64) and multi-arch ghcr.io images on `v*.*.*` tag
  push.
- HTTP CRUD routes for workspaces, members, channels, threads,
  messages (incl. tombstone via `DELETE`), mentions, votes,
  references. RFC 7807 `application/problem+json` errors via a
  custom `ApiJson` extractor.
- Event taxonomy in `maidan-types`: `Event` enum
  (`WorkspaceCreated`, `MemberJoined`, `ChannelCreated`,
  `ThreadCreated`, `MessagePosted`, `MessageTombstoned`,
  `MentionRecorded`, `VoteCast`, `ReferenceAdded`,
  `ArtifactUpserted`), `EventKind`, `EventFilter`.
- `maidan-bus::EventBus` async trait, `InMemoryBus` (tokio
  broadcast), `PostgresBus` (`LISTEN`/`NOTIFY` with a 7990-byte
  payload cap and `BusError::PayloadTooLarge`).
- Every HTTP mutation publishes the corresponding event after the
  store commit succeeds; publish errors are logged but do not turn
  successful mutations into 5xx.
- `GET /ws/subscribe` WebSocket endpoint with filter handshake,
  30 s ping / 60 s pong-timeout, bounded 256-cap mpsc backpressure,
  documented close codes (1000, 1002, 1008, 1011).
- `maidan-mcp` crate: transport-agnostic JSON-RPC 2.0 dispatcher
  supporting `initialize`, `tools/list`, `tools/call`,
  `resources/list`, `resources/read`.
- 7 MCP tools (`list_channels`, `list_threads`, `list_messages`,
  `post_message`, `record_mention`, `cast_vote`, `add_reference`)
  with typed input schemas.
- 3 MCP resource URI patterns (`maidan://workspaces/{id}`,
  `maidan://channels/{id}`, `maidan://threads/{id}`).
- `POST /mcp` HTTP endpoint wraps the MCP dispatcher.
- Integration tests: HTTP CRUD on both backends, event emission
  end-to-end, WS subscription with filters + bad-handshake close,
  MCP full flow + parse error.

### Changed

- `AppState::new` signature gained an `Arc<dyn EventBus>` parameter.
- `axum` now uses the `ws` feature.
- `docker/Dockerfile.db` no longer bundles schema into
  `docker-entrypoint-initdb.d` — the server's migration runner is
  the single source of truth.
- `deny.toml`: `allow-wildcard-paths = true` to permit workspace
  path deps; transitive testcontainers advisories
  (`RUSTSEC-2025-0134`, `RUSTSEC-2025-0111`) explicitly ignored
  with rationale.
- Every workspace crate now sets `publish.workspace = true` and
  the workspace inherits `publish = false`.

### Security

- `trufflehog --only-verified` runs on every PR in CI.
- `cargo-deny` runs on every PR in CI.
- Branch protection on `main` now requires the 5 CI jobs to pass.

## [0.0.1] — 2026-05-22

First tagged release. End of Cluster A. The repo is now a working
substrate: it builds, tests, deploys via Docker and Kubernetes, and
exposes a `/health` endpoint backed by Postgres or SQLite.

### Added

- MIT LICENSE, CONTRIBUTING.md, SECURITY.md, CHANGELOG.md,
  `.gitignore`, `.editorconfig`, `rust-toolchain.toml` (pinned to 1.88).
- Cargo workspace with 13 crates:
  `maidan-types`, `maidan-store`, `maidan-bus`, `maidan-search`,
  `maidan-fsm`, `maidan-router`, `maidan-auth`, `maidan-artifacts`,
  `maidan-mcp`, `maidan-a2a`, `maidan-observability`, `maidan-cli`,
  `maidan-server`.
- Core domain schema 0001 (workspaces, members, channels, threads,
  messages, mentions, votes, references, artifacts, audit) in both
  Postgres and SQLite dialects.
- `maidan-store::Store` async trait, `PostgresStore`, `SqliteStore`,
  `Dialect::from_url` runtime routing, idempotent migration runner.
- `maidan-artifacts::ArtifactStore` async trait, `Sha256` newtype,
  `LocalFsStore` with sha-derived fanout paths, atomic tempfile-and-
  rename writes, content-addressed dedup.
- `maidan-server`: env-driven `Config`, `AppState` over
  `Arc<dyn Trait>` handles, `/health` endpoint returning structured
  `{status, db, storage, version}` body (200 on healthy, 503 on
  degraded with per-subsystem error string), axum + tower-http
  tracing layer, migration-on-boot.
- Production multi-stage Dockerfile (cargo-chef + distroless runtime),
  `Dockerfile.dev` (cargo-watch hot reload), `docker/Dockerfile.db`
  (pgvector + bundled schema).
- `compose.yaml` (prod-style) and `compose.dev.yaml` (hot reload).
- Full Kustomize manifest set: `k8s/base/` + `overlays/dev/` +
  `overlays/prod/`.
- Obsidian docs vault under [`docs/`](docs/) with Architecture,
  Roadmap, Conventions, Deploy, Glossary, Capabilities,
  Clusters/Cluster A, Retros/Cluster A.
- Integration test suite: testcontainers-backed Postgres roundtrip,
  SQLite roundtrip (shared assertion body), cross-dialect parity
  scenario, `/health` end-to-end test, LocalFsStore roundtrip +
  concurrency stress + proptest property tests.

### Changed

- Toolchain pinned at 1.88 (forced by transitive deps `icu_*` and
  `idna`).

### Security

- Established [SECURITY.md](SECURITY.md) reporting flow (GitHub private
  advisories preferred).
- `cargo deny` allowlist + `trufflehog` scan documented in
  [`docs/Conventions.md`](docs/Conventions.md) (CI gating lands in the
  next PR).
- `k8s/base/secret.example.yaml` documents the required Secret shape
  without storing values.

[Unreleased]: https://github.com/david-engelmann/maidan/compare/v1.4.0...HEAD
[1.4.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.4.0
[1.3.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.3.0
[1.2.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.2.0
[1.1.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.1.0
[1.0.0]: https://github.com/david-engelmann/maidan/releases/tag/v1.0.0
[0.7.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.7.0
[0.6.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.6.0
[0.5.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.5.0
[0.4.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.4.0
[0.3.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.3.0
[0.2.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.2.0
[0.1.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.1.0
[0.0.1]: https://github.com/david-engelmann/maidan/releases/tag/v0.0.1
