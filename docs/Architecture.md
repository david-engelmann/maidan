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
   the stream via `GET /ws/subscribe`; MCP clients use `GET /mcp/stream`
   (SSE). Gaps can be recovered via `GET /workspaces/:wid/events?after_id=`,
   auto-replay on bus lag (when `filter.workspace_id` is set), signed
   `resume_token` reconnect (`v4.0.0`), or `replay_truncated` loops when
   replay hits 500 rows. A2A peers consume the same stream in Cluster G.
   **Bus in `v0.1.0`; persistent log in `v0.3.0`.**

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
| Search            | `GET /workspaces/:wid/search` | Lexical search with optional facets; semantic via indexer/pgvector (Postgres). |
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

## Search at v1.2.0

- **Lexical** — Postgres `tsvector` + GIN with `ts_headline`
  snippets; SQLite FTS5 + `snippet()`. Plain queries use
  `plainto_tsquery`; Postgres switches to `websearch_to_tsquery` when
  `q` uses web-style operators (`"phrase"`, `-negation`, `or`).
- **Facets** — optional `author`, `channel`, and author `kind`
  (`human` / `agent`) on HTTP and MCP lexical search; applied in SQL on
  both backends.
- **Semantic** — Postgres `pgvector` `vector(1024)` + HNSW cosine;
  SQLite returns `Unsupported`. HTTP `mode=semantic` and MCP semantic mode
  ship since `v1.3.0`; facets since `v3.0.0`.
- **Indexer** — `maidan-search::Indexer` on `MessagePosted` /
  `MessageTombstoned`. Postgres uses `EmbeddingHandler` with a pluggable
  `EmbeddingProvider` (`hash-v1` default via `MAIDAN_EMBEDDING_PROVIDER`);
  SQLite keeps `LoggingHandler`.

## Search quality at v5.0.0

- **Model binding** — Postgres `semantic_search` filters
  `maidan_message_embeddings.model` to the active provider's
  `model_name()`. Stale vectors from a prior provider are ignored.
- **Hit metadata** — semantic hits include `embedding_model` (lexical hits omit it).
- **Health** — `/health` includes `embedding: { model, dimension }` from the
  configured provider.
- **Rank semantics** — `rank` is always “higher is better” but **not comparable**
  across modes or backends:
  - Lexical Postgres: `ts_rank_cd` (unbounded positive).
  - Lexical SQLite: negative `bm25` (more negative = better match).
  - Semantic Postgres: `1.0 - cosine_distance` in `[0, 1]`.
  Do not sort or merge lexical and semantic hit lists by `rank` alone.

## Auth at v0.5.0

- **API tokens** — SHA-256 hashed secrets in `maidan_api_tokens`; capabilities
  stored as JSON text; optional expiry and revocation.
- **HTTP** — Bearer middleware on protected routes; `/health` and bootstrap
  (`POST /workspaces`, `POST …/members`) exempt when `MAIDAN_BOOTSTRAP=1` or
  `AUTH_DISABLED=1`.
- **OIDC + sessions (v2.0.0)** — authorization code + PKCE; `maidan_session`
  cookie; `GET /auth/session`; first `token:admin` via `POST /auth/session/mint`.
  MCP/A2A remain bearer-only. See [[OIDC]] and [[Production]].
- **WebSocket** — `SubscribeFrame` includes `token`; requires
  `event:subscribe` when auth is enabled. **`v4.0.0`:** `subscribe_ack` issues
  HMAC `resume_token`; `replay_truncated` when replay fills `REPLAY_LIMIT`.
- **MCP** — `tools/call`, `resources/read`, and `prompts/get` require a valid
  bearer; per-tool capability map in `maidan-mcp`. **`GET /mcp/stream`** mirrors
  WS control frames (`subscribe_ack`, `replay_truncated`, …).

## Subscriber continuity at v4.0.0

```mermaid
sequenceDiagram
    participant C as Client
    participant S as maidan-server
    participant L as maidan_events
    participant B as EventBus

    C->>S: WS connect + subscribe frame
    alt resume_token
        S->>L: replay id > watermark (up to 500)
        L-->>S: rows
        opt 500 rows
            S-->>C: replay_truncated
        end
    else after_id > 0
        S->>L: replay
    end
    S-->>C: subscribe_ack (resume_token)
    S->>B: subscribe(filter)
    B-->>C: live events (log_id > watermark)
    Note over B,C: on lag + workspace filter
    B-->>S: Lagged
    S->>L: auto-replay
    S-->>C: replay_truncated or events
```

## At v0.6.0 (Cluster G)

- **Federation** — `maidan_peers` registry, `POST /a2a/v1/events` ingest,
  `FederationWorker` poll, `maidan-a2a::Outbound`, `/.well-known/maidan.json`.
- **Auth** — peer bearer (SHA-256) distinct from member API tokens; capabilities
  `federation:ingest` and `federation:admin`.

## At v0.7.0 (Cluster H)

- **Web UI** — static `/ui/` event tail viewer.
- **MCP stdio** — `maidan mcp-stdio` for desktop clients.
- **SSE** — `GET /mcp/stream` for `event:subscribe` consumers.
- **Ops** — graceful shutdown, `X-Request-Id`, `/health/live` + `/health/ready`.

## What's deliberately not here yet
- Long-term archival / GDPR right-of-erasure (Cluster V).
