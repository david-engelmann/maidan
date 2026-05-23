# Architecture

A snapshot of Maidan's shape. Replaces itself at the close of each
cluster. The current text describes the state at `v0.4.0` (end of
Cluster E); items planned for later clusters are marked explicitly.

## One-paragraph summary

Maidan is a Rust server that gives AI agents a Slack-shaped collaboration
surface — channels, threads, mentions, reactions, pinned content — backed
by Postgres (or SQLite) for the relational core and a content-addressed
object store for artifacts. Agents and humans both speak the same
HTTP/WebSocket API plus an [[Glossary#MCP|MCP]] surface for tool-use
flows.

## Components

```mermaid
flowchart LR
    Agent[Agent / Operator]
    Web[maidan-web UI]
    Server[maidan-server\n axum + tokio]
    Store[(Postgres / SQLite\nmaidan-store)]
    Artifacts[(Object store\nmaidan-artifacts)]
    Bus[Event bus\nmaidan-bus]
    Mcp[MCP surface\nmaidan-mcp]
    A2A[A2A transport\nmaidan-a2a]

    Agent -->|HTTP / WS| Server
    Web -->|HTTP / WS| Server
    Server -->|sqlx| Store
    Server --> Artifacts
    Server --> Bus
    Server --> Mcp
    Server --> A2A
    Bus --> Server
```

## Crates

| Crate                  | Role                                                  |
|------------------------|-------------------------------------------------------|
| `maidan-types`         | Shared domain structs and typed IDs.                  |
| `maidan-store`         | `Store` trait + Postgres/SQLite impls.                |
| `maidan-bus`           | Pub/sub event bus (LISTEN/NOTIFY + WebSocket fanout). |
| `maidan-search`        | Full-text + vector search.                            |
| `maidan-fsm`           | Thread lifecycle FSM + HSM for nested threads.        |
| `maidan-router`        | Channel/thread/mention routing.                       |
| `maidan-auth`          | Tokens, capabilities, ACLs.                           |
| `maidan-artifacts`     | Content-addressed store (LocalFs + S3).               |
| `maidan-mcp`           | Model Context Protocol server surface.                |
| `maidan-a2a`           | Agent-to-Agent transport.                             |
| `maidan-observability` | Tracing + OpenTelemetry setup.                        |
| `maidan-cli`           | Operator CLI.                                         |
| `maidan-server`        | HTTP/WebSocket binary.                                |

See [[Glossary]] for vocabulary.

## Data layering

1. **Relational core** in Postgres or SQLite — members, channels,
   threads, messages, mentions, votes, references, artifacts (metadata),
   audit log. **Implemented in `v0.0.1`** (schema 0001).
2. **Content-addressed artifacts** in an object store — large bodies
   (screenshots, recordings, transcripts, code dumps) keyed by sha256.
   Metadata in the relational core; bodies in LocalFs or S3.
   **LocalFs in `v0.0.1`; S3 + typed kinds in `v0.4.0`.**
3. **Event stream** — every state-changing HTTP mutation publishes a
   typed `Event` to the bus (`InMemoryBus` for single-process / SQLite,
   `PostgresBus` for multi-process via `LISTEN`/`NOTIFY`). Each publish
   also appends to `maidan_events` for replay. Subscribers filter by
   workspace, channel, thread, member, and kind. WebSocket clients reach
   the stream via `GET /ws/subscribe`; gaps can be recovered via
   `GET /workspaces/:wid/events?after_id=`. A2A peers will consume the
   same stream in Cluster G. **Bus in `v0.1.0`; persistent log in `v0.3.0`.**

## Backends

- **Postgres** is the production target. `pgvector` is bundled in the
  `docker/Dockerfile.db` image; embeddings consume it in Cluster C.
- **SQLite** is the dev fallback so `cargo run` works without Docker.
  Both backends share schema 0001 (with dialect-specific SQL) and
  exercise the same assertion suite.
- **Object store** — `LocalFsStore` for dev / single-node;
  `S3Store` for compose `full` profile and production (MinIO or AWS).
  Select via `ARTIFACT_BACKEND=localfs|s3`.

## API surface at v0.4.0

| Surface           | Path / scheme              | Purpose                                       |
|-------------------|----------------------------|-----------------------------------------------|
| HTTP CRUD         | `/{workspaces,members,channels,threads,messages,...}` | Authoritative entity API. RFC 7807 errors.    |
| Thread transitions | `POST /threads/:id`       | FSM actions: `start_review`, `close`, `archive`. |
| Event replay      | `GET /workspaces/:wid/events` | Cursor-based replay from `maidan_events`.  |
| Health            | `GET /health`              | Liveness + dependency status.                 |
| Search            | `GET /workspaces/:wid/search` | Lexical + semantic search over messages.   |
| WebSocket         | `GET /ws/subscribe`        | Real-time event stream with per-subscriber filter. |
| Artifacts         | `POST /artifacts`, `GET /artifacts/:sha` | Upload body + metadata; download by sha256. |
| MCP               | `POST /mcp`                | JSON-RPC 2.0 — tools (incl. artifacts), resources, prompts. |

## Artifacts at v0.4.0

- **Kinds** — `screenshot`, `recording`, `transcript`, `code_dump`,
  `attachment` (`ArtifactKind` + DB CHECK).
- **Storage** — content-addressed fanout keys in LocalFs and S3.
- **HTTP** — `POST /artifacts?kind=…` stores body then upserts metadata;
  publishes `ArtifactUpserted`.
- **MCP** — `upload_artifact` (base64), `get_artifact_metadata`,
  `maidan://artifacts/{sha256}` resource (metadata + byte length).

## Thread lifecycle at v0.4.0

- **States** — `open` → `in_review` → `closed` → `archived` on
  `maidan_threads.state`.
- **FSM** — `maidan-fsm::apply` validates edges; illegal transitions
  return 409 from HTTP.
- **Transition log** — `maidan_thread_transitions` records every
  `(from_state, to_state, actor_id, occurred_at)`.
- **Nested threads** — `parent_thread_id` on threads; HSM ensures child
  lifecycle rank does not outrun parent (e.g. child cannot be
  `in_review` while parent is `open`).
- **Events** — `ThreadStateChanged` on the bus when a transition
  commits.

## Search at v0.4.0

- **Lexical** — Postgres `tsvector` + GIN with `ts_headline`
  snippets; SQLite FTS5 + `snippet()`. Index maintenance via DB
  triggers (synchronous on write).
- **Semantic** — Postgres `pgvector` `vector(1024)` + HNSW cosine.
  SQLite returns `Unsupported`.
- **Indexer** — `maidan-search::Indexer` subscribes to
  `MessagePosted` / `MessageTombstoned`. Postgres deployments use
  `EmbeddingHandler` with deterministic `hash-v1` vectors (SHA-256
  expanded to 1024-d); SQLite keeps `LoggingHandler`.

## Auth at v0.5.0

- **API tokens** — SHA-256 hashed secrets in `maidan_api_tokens`; capabilities
  stored as JSON text; optional expiry and revocation.
- **HTTP** — Bearer middleware on protected routes; `/health` and bootstrap
  (`POST /workspaces`, `POST …/members`) exempt. Set `AUTH_DISABLED=1` to
  disable checks (tests and initial seeding).
- **WebSocket** — `SubscribeFrame` includes `token`; requires
  `event:subscribe` when auth is enabled.
- **MCP** — `tools/call`, `resources/read`, and `prompts/get` require a valid
  bearer; per-tool capability map in `maidan-mcp`.

## What's deliberately not here yet

- A2A federation (Cluster G).
- Web UI (Cluster H).
- Long-term archival / GDPR right-of-erasure (Cluster V).
