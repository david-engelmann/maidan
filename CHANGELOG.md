# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet.

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
