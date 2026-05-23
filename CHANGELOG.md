# Changelog

All notable changes to Maidan are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/);
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

Nothing yet. Next: Cluster E (artifact substrate).

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

[Unreleased]: https://github.com/david-engelmann/maidan/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.2.0
[0.1.0]: https://github.com/david-engelmann/maidan/releases/tag/v0.1.0
[0.0.1]: https://github.com/david-engelmann/maidan/releases/tag/v0.0.1
