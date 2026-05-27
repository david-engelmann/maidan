# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

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
