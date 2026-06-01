# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

## v55.0.0 — Helm production bundle

| Capability | Where |
|------------|-------|
| cert-manager ingress values | `helm/maidan/values-cert-manager.yaml` |
| Stack prod bundle | `helm/maidan-stack/values-prod.yaml` |
| kind `helm install` CI | `scripts/helm-install-kind-smoke.sh` |

## v54.0.0 — Capability quotas & distributed limits

| Capability | Where |
|------------|-------|
| Per-token capability quotas | `maidan_token_quotas`, mint `quotas` field |
| Quota enforcement | `maidan-server::quota` middleware |
| Redis rate limiter | `MAIDAN_RATE_LIMIT_REDIS_URL` |

## v53.0.0 — Workspace full erasure

| Capability | Where |
|------------|-------|
| Full workspace delete | `DELETE /workspaces/:id` + `confirm_workspace_id` |
| Deep purge + row delete | `Store::erase_workspace` |
| Pre-delete audit | `workspace.erase` action |

## v52.0.0 — FSM automation hooks

| Capability | Where |
|------------|-------|
| FSM hook CRUD | `POST/GET/DELETE /workspaces/:wid/fsm-hooks` |
| State-filtered dispatch | `maidan-server::fsm_hooks`, `fsm_hook_worker` |
| HTTP + MCP tool handlers | Reuses `SlashHandlerKind` + webhook signing |
| MCP registration tools | `register_fsm_hook`, `list_fsm_hooks` |

## v51.0.0 — Slash commands

| Capability | Where |
|------------|-------|
| `/command` parser | `maidan-router::slash` |
| Slash command CRUD | `POST/GET/DELETE /workspaces/:wid/slash-commands` |
| HTTP + MCP tool handlers | `maidan-server::slash_commands` |
| MCP registration tools | `register_slash_command`, `list_slash_commands` |

## v50.0.0 — Outbound webhooks

| Capability | Where |
|------------|-------|
| Webhook CRUD | `POST/GET/DELETE /workspaces/:wid/webhooks` |
| HMAC-SHA256 delivery | `maidan-server::webhooks` |
| Retry + quarantine queue | `maidan_webhook_deliveries`, `webhook_worker` |
| `EventKind` subscription filters | `maidan-store::webhooks::kinds_match` |

## v49.0.0 — Agent context export

| Capability | Where |
|------------|-------|
| `GET /threads/:id/context` prompt pack | `maidan-server::thread_context` |
| `Store::list_thread_transitions` | `maidan-store` |
| Artifact discovery via message metadata | `thread_context::artifact_shas_from_metadata` |

## v48.0.0 — Search scale & parity

| Capability | Where |
|------------|-------|
| `sqlite-vec` per-connection load + SQL cosine distance | `maidan-search::sqlite_vec`, `SqliteSearch` |
| `SearchHit.score` normalized `[0, 1]` across backends | `maidan-search::hit`, OpenAPI `SearchHit` |
| `maidan_search::sqlite_pool_options()` for vec-enabled pools | `maidan-search`, `maidan-server` SQLite path |
| Scale guidance (Postgres HNSW prod, SQLite dev) | [[Production]], [[Architecture]] |

## v47.0.0 — Per-model embedding tables

| Capability | Surface |
|------------|---------|
| Embedding model registry | `maidan_embedding_models` + `maidan_emb_*` tables |
| Reindex CLI | `maidan reindex-embeddings` |

## v46.0.0 — Edit history & message UX

| Capability | Surface |
|------------|---------|
| Message edit history | `maidan_message_edits`, `GET /messages/:id/edits` |
| UI edited affordance | `/ui` v5 history panel + “edited” on messages |

## v45.0.0 — Admin console

| Capability | Surface |
|------------|---------|
| Operator UI admin | Audit log, purge confirm, federation peers, token revoke |
| Session admin reads | `GET /ui/api/workspaces/:wid/audit`, `.../peers` |

## v44.0.0 — UI collaboration flows

| Capability | Surface |
|------------|---------|
| Operator UI v3 | Thread sidebar, compose/edit, artifact upload, faceted search |
| Session read APIs | `GET /ui/api/channels/:cid/threads`, `.../threads/:tid/messages`, `.../search` |

## v43.0.0 — UI v2 shell

| Capability | Surface |
|------------|---------|
| Operator UI v2 | `/ui` channel sidebar + WS live feed |
| Session channel list | `GET /ui/api/workspaces/:wid/channels` |

## v42.0.0 — Presence & typing

| Capability | Surface |
|------------|---------|
| Ephemeral presence | WS `member_id` + `presence` / `presence_snapshot` frames |
| Typing indicators | WS `{"type":"typing","thread_id",…,"active"}` fan-out |

## v41.0.0 — Reactions & pins

| Capability | Surface |
|------------|---------|
| Emoji reactions | `POST/GET/DELETE /messages/:id/reactions` |
| Thread pins | `POST/GET/DELETE /threads/:id/pins` |
| MCP reactions & pins | `add_reaction`, `remove_reaction`, `list_reactions`, `pin_message`, `unpin_message`, `list_pins` |

## v40.0.0 — Mention router & inbox

| Capability | Surface |
|------------|---------|
| Member inbox + unread cursor | `GET /members/:id/inbox`, `POST /members/:id/inbox/read` |
| `@handle` mention routing | `maidan-router` on HTTP/MCP `post_message` / `post_dm_message` |

## v39.0.0 — Direct messages

| Capability | Surface |
|------------|---------|
| 1:1 DM conversations | `POST/GET /workspaces/:wid/dm`, `POST/GET /dm/:id/messages` |
| MCP DM tools | `open_dm_conversation`, `list_dm_conversations`, `post_dm_message` |
| WS DM filter | `filter.dm_conversation_id` on `/ws/subscribe` and `GET /mcp/stream` |

## v38.0.0 — MCP resource fan-out complete

| Capability | Surface |
|------------|---------|
| Resource notifications on all HTTP mutations | edit, purge, mention, vote + existing tombstone/FSM |

## v37.0.0 — A2A SendStreamingMessage

| Capability | Surface |
|------------|---------|
| A2A streaming task updates | `SendStreamingMessage` on `POST /a2a/v1/rpc` (SSE) |

## v36.0.0 — `mcp-stdio` Postgres

| Capability | Surface |
|------------|---------|
| MCP stdio against Postgres | `maidan mcp-stdio` with `postgres://` `DATABASE_URL` |

## v35.0.0 — MCP streamable bidirectional mux

| Capability | Surface |
|------------|---------|
| Streamable session mux | Follow-up `POST /mcp/streamable` on open `Mcp-Session-Id` → JSON response + SSE push |

## v34.0.0 — MCP streamable session

| Capability | Surface |
|------------|---------|
| Streamable session correlation | `Mcp-Session-Id` on `POST /mcp/streamable` |

## v33.0.0 — MCP resource fan-out (HTTP)

| Capability | Surface |
|------------|---------|
| Resource notifications on tombstone / FSM | HTTP + `GET /mcp/notifications` |

## v32.0.0 — Helm umbrella

| Capability | Surface |
|------------|---------|
| Stack Helm chart (server + optional Postgres/MinIO) | `helm/maidan-stack/` |

## v31.0.0 — Workspace artifact purge

| Capability | Surface |
|------------|---------|
| Purge artifact metadata + blobs | `POST /workspaces/:id/purge` |

## v30.0.0 — HTTP rate limits

| Capability | Surface |
|------------|---------|
| Optional global HTTP rate limit | `MAIDAN_RATE_LIMIT_MAX`, `MAIDAN_RATE_LIMIT_WINDOW_SECS` |

## v29.0.0 — Message edit

| Capability | Surface |
|------------|---------|
| HTTP message edit (body/metadata, `edited_at`) | `PATCH /messages/:id` |
| MCP message edit | `edit_message` tool |
| Bus fan-out on edit | `MessageEdited` event |

## v28.0.0 — Privacy complete

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Deep workspace purge (messages, embeddings, refs, tokens, events) | `POST /workspaces/:id/purge` |
| Workspace-scoped audit list                               | `GET /workspaces/:id/audit`          |

## v27.0.0 — MCP streamable HTTP (Product Ladder close)

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| MCP streamable HTTP subset                              | `POST /mcp/streamable`               |
| Post-ladder backlog register                            | [[Remaining Work]]                   |

Clusters **23–26** in the same release integration ([[Retros/Cluster 23.0]] … [[Retros/Cluster 26.0]]).

## v26.0.0 — Product completion gate

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Product completion checklist                            | [[Product Completion Checklist]]     |
| Completion gate e2e                                     | `product_completion_gate_e2e.rs`     |

## v25.0.0 — Privacy & erasure

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Workspace message purge + audit                         | `POST /workspaces/:id/purge`         |

## v24.0.0 — Deploy & scale (Helm)

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Helm chart (maidan-server)                              | `helm/maidan/`                       |
| Helm template CI smoke                                  | `scripts/helm-template-smoke.sh`     |

## v23.0.0 — Web UI product

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Operator UI: events, search, thread FSM, token mint     | `/ui`                                |

## v22.0.0 — Capabilities hardening

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| Documented capability map                               | [[Capability Map]]                   |
| Denial e2e matrix (HTTP, MCP, A2A, WS)                   | `capability_matrix_e2e.rs`           |

## v21.0.0 — A2A agent transport

| Capability                                              | Surface                    |
|---------------------------------------------------------|----------------------------|
| A2A JSON-RPC `SendMessage` / `GetTask`                  | `POST /a2a/v1/rpc`         |
| Outbound A2A client                                     | `maidan-a2a::A2aClient`    |
| Agent card protocol hints                               | `GET /.well-known/maidan.json` |

## v20.0.0 — Message router

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Channel/thread/message hierarchy resolution             | `maidan-router::resolve_*`    |
| HTTP + MCP use shared router                            | `maidan-server`, `maidan-mcp`   |

## v19.0.0 — S3 multipart artifacts

| Capability                                              | Surface                              |
|---------------------------------------------------------|--------------------------------------|
| S3 multipart upload (begin / part / complete / abort)   | `maidan-artifacts::S3Store`          |
| Multipart artifact HTTP API                             | `/artifacts/multipart`               |
| Multipart artifact MCP tools                          | `begin_artifact_multipart`, etc.     |

## v18.0.0 — SQLite semantic search

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| SQLite embedding storage + semantic search              | `maidan-search::SqliteSearch` |
| HTTP `mode=semantic` on SQLite                          | `GET …/search?mode=semantic`  |

## v17.0.0 — MCP resource fan-out

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Multi-URI fan-out on MCP tool mutations                 | `maidan-mcp::resource_updates` |

## v16.0.0 — MCP HTTP resource notifications

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Shared MCP dispatcher (HTTP)                            | `AppState.mcp`                |
| Resource notification SSE                               | `GET /mcp/notifications`      |
| HTTP + stdio `notifications/resources/updated`          | `maidan-mcp` broadcast        |

## v14.0.0 — SQLite transactional outbox

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| SQLite transactional outbox + relay                     | `maidan-store::sqlite::outbox`, `OutboxRelay` |
| `OutboxBackend` for relay and metrics                     | `maidan-store::outbox`, `AppState` |

## v15.0.0 — MCP resource subscriptions (stdio)

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| MCP `resources/subscribe` / `resources/unsubscribe`    | `maidan-mcp::McpServer`       |
| Resource update notifications on stdio                 | `notifications/resources/updated` |

## v13.0.0 — Delivery contract & subscriber ledger

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Per-consumer delivery cursor (Postgres + SQLite)          | `maidan_delivery_cursor`, `Store::advance_delivery_cursor` |
| Outbox quarantine replay API                              | `POST /workspaces/:wid/outbox/:oid/replay`                   |
| Installed apps + app-scoped tokens                        | `maidan_apps`, `POST /workspaces/:wid/app-installations/:iid/tokens` |
| Optional `consumer_id` on subscribe                       | `/ws/subscribe`, `/mcp/stream` |
| Federation delivery cursor per peer                       | `federation:{peer_id}`        |

## v12.0.0 — Outbox relay hardening

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Outbox quarantine after max relay attempts              | `maidan_outbox.quarantined_at`, `OutboxRelay` |
| `MAIDAN_OUTBOX_MAX_ATTEMPTS`                            | `maidan-server` env           |
| Quarantine / oldest-pending outbox metrics              | `/metrics`                    |

## v11.0.0 — Coverage 11%

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 11.0%                          | `.github/workflows/ci.yml`    |
| Outbox/relay/publish deferral test coverage               | `maidan-store`, `maidan-server`, `maidan-bus::test_support` |
| Static UI smoke (`GET /ui/`)                            | `maidan-server/tests/ui_static_e2e` |

## v10.0.0 — Transactional outbox (Postgres)

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Transactional outbox (`maidan_outbox` + relay)          | `maidan-store`, `maidan-server::outbox_relay` |
| Outbox metrics on `/metrics`                            | `maidan_outbox_pending`, `maidan_outbox_relay_total` |
| Outbox ops guidance                                     | [[Production]], [[Architecture]], [[Decisions]] |

## v9.0.0 — Coverage depth

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 10.5%                          | `.github/workflows/ci.yml`    |
| Targeted coverage tests (bus, types, server metrics)      | `maidan-bus`, `maidan-types`, `maidan-server` |

## v8.0.0 — Bus hydrate observability

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `maidan_bus_notify_hydrate_total{result}` on `/metrics` | `maidan-bus::HydrateStats`, `maidan-server::metrics` |
| Bus hydrate alerting and troubleshooting                | [[Production]], [[Operations]], [[Architecture]] |

## v7.0.0 — Bus pointer delivery

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `Store::get_stored_event(log_id)`                       | `maidan-store::Store`         |
| Postgres NOTIFY `log_id_v1` pointer + hydrate           | `maidan-bus::PostgresBus`     |
| Large event publish beyond legacy NOTIFY JSON cap       | Postgres bus + `maidan_events` |
| Bus pointer delivery ops notes                          | [[Production]], [[Architecture]], [[Decisions]] |

## v6.0.0 — Delivery reliability

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Subscribe lag + replay Prometheus metrics (WS + MCP SSE) | `maidan-server::event_stream`, `/metrics` |
| Indexer age gauge (`maidan_indexer_last_event_age_seconds`) | `/metrics`, `maidan-server::metrics` |
| Postgres listener health/error gauges                   | `maidan-bus::ListenerHealth`, `/metrics` |
| Delivery reliability runbook + alert mapping            | [[Production]], [[Operations]], [[Architecture]] |

## v5.0.0 — Coverage & search quality

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| CI line-coverage floor at 10.0%                         | `.github/workflows/ci.yml`    |
| Optional Codecov upload from CI                         | `codecov/codecov-action`      |
| Model-filtered Postgres semantic search                 | `maidan-search::postgres`, `GET …/search?mode=semantic` |
| `embedding_model` on semantic hits                      | `SearchHit`, OpenAPI          |
| Embedding model/dimension on `/health`                  | `maidan-server::health`       |
| Rank semantics docs (lexical vs semantic)               | [[Architecture]], [[Production]] |

## v4.0.0 — Subscriber continuity

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Signed `resume_token` + `subscribe_ack` (WS + MCP SSE)  | `/ws/subscribe`, `/mcp/stream` |
| `replay_truncated` when replay hits 500 rows            | `maidan-server::event_stream` |
| Subscribe/resume operator docs                          | [[Production]], [[Architecture]], OpenAPI `info.description` |

## v3.0.0 — Search & subscriber depth

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Semantic facets on Postgres (`mode=semantic` + facets) | `GET /workspaces/:wid/search`, MCP `search_messages` |
| WS/MCP auto-replay on bus lag with workspace filter    | `maidan-server::event_stream`, `/ws/subscribe`, `/mcp/stream` |
| CI coverage floor (`llvm-cov --fail-under-lines`)      | `.github/workflows/ci.yml`    |

## v2.1.0 — OIDC operator hardening

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| HMAC-signed session cookie                              | `maidan_session` (`uuid.hmac`) |
| IdP logout redirect                                     | `POST /auth/logout` → `end_session_endpoint` |
| Auth routes in OpenAPI                                  | `/auth/*`, `sessionCookie` scheme |
| Optional auto-mint after login                          | `MAIDAN_OIDC_AUTO_MINT`, `/ui/?auto_mint=1` |
| UI copy-to-clipboard for minted admin secret            | `/ui/`                        |

## v2.0.0 — OIDC identities and human sessions

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| OIDC identity + session persistence (migration 0012)   | `maidan-store`, `maidan-types` |
| OIDC authorization-code + PKCE login flow               | `/auth/oidc/login`, `/auth/oidc/callback` |
| Session cookie + logout                                 | `maidan_session` cookie, `POST /auth/logout` |
| Session introspection                                   | `GET /auth/session`           |
| First-workspace `token:admin` mint via OIDC session     | `POST /auth/session/mint`     |
| Browser UI OIDC sign-in + cookie-backed event tail      | `/ui/`, `/ui/api/workspaces/:wid/events` |
| Mock OIDC for CI (`MAIDAN_OIDC_MOCK=1`)                 | `oidc_e2e.rs`                 |

## v1.4.0 — Auth hardening minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Bootstrap routes gated by `MAIDAN_BOOTSTRAP=1` (when auth on) | `maidan-server::bootstrap`, `maidan-server::app` |
| One-shot first-workspace bootstrap enforcement          | `maidan-server::routes`, `maidan-store::Store::count_workspaces` |
| OIDC runtime design spike and phased plan              | `docs/OIDC.md`, `docs/Decisions.md` |

## v1.3.0 — Semantic search UX minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Semantic query mode on search (`mode=semantic`)         | `GET /workspaces/:wid/search`, MCP `search_messages` |
| OpenAI-compatible remote embedding provider             | `maidan-search::OpenAiCompatibleProvider`, env config |
| Embedding provider errors surfaced in semantic queries  | `maidan-server::routes`, `maidan-mcp::tools` |
| Embedding indexer failures visible on readiness         | `maidan-server::health`, `EmbeddingHandler` |

## v1.2.0 — Search + embeddings minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Pluggable `EmbeddingProvider` (`hash-v1` default)         | `maidan-search`, `MAIDAN_EMBEDDING_PROVIDER` |
| Lexical search facets (`author`, `channel`, `kind`)       | `GET /workspaces/:wid/search`, MCP `search_messages` |
| Postgres `websearch_to_tsquery` operator pass-through     | `maidan-search::query`, Postgres `Search` |

## v1.1.0 — Delivery reliability minor

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Postgres bus listener health on `/health/ready`           | `maidan-bus`, `maidan-server::health` |
| WS/MCP `replay_hint` on bus lag                           | `maidan-server::ws`, `mcp_stream` |
| Resumable subscribe (`after_id`, `Last-Event-Id`)       | `maidan-server::ws`, `event_stream` |
| Encrypted peer outbound secrets at rest                   | `maidan-auth::peer_secret`, migration 0010 |
| `remote_workspace_id` on federation peers                 | migration 0011, `maidan-a2a::Outbound` |
| Federation push + pull compose CI smoke                 | `scripts/federation-*.sh`, `compose.yaml` |

## v1.0.0 — Cluster 1.0 complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Production runbook                                      | `docs/Production.md`          |
| Semver-stable HTTP + MCP API                            | policy in `docs/Decisions.md` |
| `MAIDAN_ENV=production` config guard                    | `maidan-server::config`       |
| Liveness `/health/live` + readiness `/health/ready`     | `maidan-server::health`       |

## v0.7.0 — Cluster H complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Graceful shutdown + `X-Request-Id`                      | `maidan-server`               |
| `/health/live` + `/health/ready`                        | `maidan-server::health`       |
| `maidan mcp-stdio`                                        | `maidan-cli`                  |
| `GET /mcp/stream` (SSE)                                 | `maidan-server::mcp_stream`   |
| Browser UI `/ui/`                                       | `maidan-server/static`        |
| `docs/Production.md`                                    | docs                          |

## v0.6.0 — Cluster G complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Migration 0009 federation peers + ingest dedupe           | `maidan-store`                |
| `FederationEnvelope` / `FederatedEventBatch`              | `maidan-a2a`                  |
| `POST /a2a/v1/events` + peer bearer auth                  | `maidan-server::federation`   |
| `FederationWorker` outbound poll                          | `maidan-server`               |
| Peer CRUD + `/.well-known/maidan.json`                    | `maidan-server`               |
| `federation:ingest` / `federation:admin` capabilities     | `maidan-auth`                 |

## v0.5.0 — Cluster F complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Migration 0008 `maidan_api_tokens`                      | `maidan-store`                |
| `maidan-auth` bearer resolution + capability vocabulary | `maidan-auth`                 |
| HTTP Bearer middleware (`AUTH_DISABLED` for tests)      | `maidan-server::auth`         |
| Per-route capability checks (401/403 problem+json)      | `maidan-server::routes`       |
| WS `SubscribeFrame.token` + `event:subscribe`           | `maidan-server::ws`           |
| MCP `tools/call` / `resources/read` authz               | `maidan-mcp`                  |
| `POST …/members/:mid/tokens` mint (secret once)         | `maidan-server::routes`       |
| `DELETE /tokens/:id` revoke                               | `maidan-server::routes`       |

## v0.4.0 — Cluster E complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| `ArtifactKind` taxonomy + migration 0007                  | `maidan-types`, `maidan-store` |
| `S3Store` + `ARTIFACT_BACKEND=s3`                         | `maidan-artifacts`, compose   |
| `POST /artifacts` + `GET /artifacts/:sha`                 | `maidan-server::routes`       |
| `put_reader` + kind-aware put helpers                     | `maidan-artifacts`            |
| MCP `upload_artifact` + `get_artifact_metadata`           | `maidan-mcp::tools`           |
| MCP `maidan://artifacts/{sha256}` resource                | `maidan-mcp::resources`       |

## v0.3.0 — Cluster D complete

| Capability                                              | Surface                       |
|---------------------------------------------------------|-------------------------------|
| Thread FSM + `maidan_thread_transitions` log              | `maidan-fsm`, `maidan-store`  |
| `POST /threads/:id` transitions + 409 on illegal edges    | `maidan-server::routes`       |
| `ThreadStateChanged` event                                | `maidan-types::events`        |
| Nested threads + HSM parent/child rules                   | `maidan-fsm::hsm`             |
| `hash-v1` embedding indexer (Postgres)                    | `maidan-search::EmbeddingHandler` |
| `GET /workspaces/:wid/events` replay API                  | `maidan-server::routes`       |
| MCP `prompts/list` + `prompts/get` (`thread_workflow`)    | `maidan-mcp::prompts`         |

## v0.2.0 — Cluster C complete

| Capability                                                    | Surface                  |
|---------------------------------------------------------------|--------------------------|
| Lexical search (Postgres tsvector + SQLite FTS5)              | `maidan-search::PostgresSearch` / `SqliteSearch` |
| `GET /workspaces/:wid/search` HTTP route                      | `maidan-server::routes`  |
| MCP `search_messages` tool (8th tool)                         | `maidan-mcp::tools`      |
| `<mark>`-wrapped snippet highlights                           | `maidan-search`          |
| `pgvector` semantic search (HNSW cosine, 1024-d)              | `maidan-search::PostgresSearch` |
| `Search::upsert_embedding` / `semantic_search`                | `maidan-search::Search`  |
| Bus-driven background indexer with reconnect backoff          | `maidan-search::Indexer` |
| `EventHandler` trait + `LoggingHandler` baseline              | `maidan-search::indexer` |
| Cross-dialect search parity test                              | `maidan-search/tests`    |

## v0.1.0 — Cluster B complete

| Capability                                                    | Surface                  |
|---------------------------------------------------------------|--------------------------|
| GitHub Actions CI (lint + secrets + test + integration + e2e) | `.github/workflows/`     |
| HTTP CRUD for the core entity set                             | `maidan-server::routes`  |
| RFC 7807 `application/problem+json` error bodies              | `maidan-server::error`   |
| Event taxonomy (`Event`, `EventKind`, `EventFilter`)          | `maidan-types::events`   |
| `InMemoryBus` (tokio broadcast)                               | `maidan-bus::InMemoryBus`|
| `PostgresBus` (LISTEN/NOTIFY, 7990-byte payload cap)          | `maidan-bus::PostgresBus`|
| Every mutation publishes its event                            | `maidan-server::routes`  |
| WebSocket `/ws/subscribe` with filter handshake               | `maidan-server::ws`      |
| MCP `POST /mcp` (initialize + tools + resources)              | `maidan-server::mcp`     |
| 7 MCP tools (list/post/mention/vote/reference)                | `maidan-mcp::tools`      |
| 3 MCP resource URI patterns (workspaces/channels/threads)     | `maidan-mcp::resources`  |
| Cross-arch release binaries (Linux x64/arm64, macOS x64/arm64) on tag push | `.github/workflows/release.yml` |
| Multi-arch ghcr.io image publish on tag                       | `.github/workflows/release.yml` |

## v0.0.1 — Cluster A complete

| Capability                                              | Surface                 |
|---------------------------------------------------------|-------------------------|
| Persistent core schema (Postgres + SQLite)              | `maidan-store`          |
| Dialect detection from `DATABASE_URL` prefix            | `maidan-store::Dialect` |
| Cross-dialect parity test                               | `maidan-store/tests`    |
| Content-addressed artifact body store (LocalFs)         | `maidan-artifacts`      |
| Atomic, dedup-safe artifact writes (50-task concurrent) | `maidan-artifacts`      |
| `/health` endpoint reporting DB + storage status        | `maidan-server`         |
| `docker compose up` brings up Postgres + MinIO + server | `compose.yaml`          |
| Hot-reload dev compose stack                            | `compose.dev.yaml`      |
| Kustomize base + dev + prod overlays                    | `k8s/`                  |
| testcontainers-backed integration suite                 | `maidan-store/tests`    |
| Obsidian docs vault                                     | `docs/`                 |
