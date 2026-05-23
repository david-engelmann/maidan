# Cluster B retro — Routing + event bus + MCP

> Closing wave for Cluster B · target tag `v0.1.0`.

Cluster A delivered a substrate; Cluster B turned it into something
agents and humans can actually talk to. Five PRs (six counting this
retro) landed in two working days.

## What shipped

- **PR #20** — `ci: github actions workflows` — `lint` (fmt + clippy +
  cargo-deny), `secrets` (trufflehog), `test` (unit), `integration`
  (testcontainers Postgres + in-memory SQLite), `e2e` (docker compose
  + `/health` smoke). All five required-status-checks on `main`. Plus
  nightly mutation/bench skeleton and a release workflow that builds
  cross-arch binaries + multi-arch ghcr.io images on tag push.
- **PR #21** — `feat(maidan-server): http crud api for core entities` —
  REST routes for workspaces, members, channels, threads, messages,
  mentions, votes, references. RFC 7807 `application/problem+json`
  error bodies via a custom `ApiJson` extractor.
- **PR #22** — `feat(maidan-bus): postgres listen/notify + in-memory
  backends` — Event taxonomy in `maidan-types`, `EventBus` trait,
  `InMemoryBus` (tokio broadcast), `PostgresBus` (LISTEN/NOTIFY with
  full-JSON payload + 7990-byte cap). Every HTTP mutation publishes
  the matching event.
- **PR #23** — `feat(maidan-server): websocket /ws/subscribe` —
  upgrade + filter handshake + JSON event frames + 30 s ping/60 s
  pong-timeout + bounded 256-cap mpsc backpressure. Documented close
  codes (1000/1002/1008/1011).
- **PR #24** — `feat(maidan-mcp): mcp server surface` —
  transport-agnostic JSON-RPC 2.0 dispatcher with `initialize`,
  `tools/list` (7 tools), `tools/call`, `resources/list` (3 URI
  patterns), `resources/read`. Wired behind `POST /mcp`.

## What was deferred

| To             | What                                                       | Why                                                                |
|----------------|------------------------------------------------------------|--------------------------------------------------------------------|
| Cluster T      | Coverage upload (cargo-llvm-cov + codecov)                 | Tooling lands when the coverage gate tightens.                     |
| Cluster T      | OTLP exporter, request-id middleware, JSON logs            | Observability surface is its own cluster.                          |
| Cluster T      | Persistent event log table (id-pointer + table fetch)      | Mitigates NOTIFY 7990-byte cap when it bites; not bitten yet.      |
| Cluster D      | Resumable WS subscriptions / reconnection tokens           | Needs the FSM cluster.                                             |
| Cluster D      | MCP `prompts/list` + `prompts/get`                         | Wait for thread-prompt model.                                      |
| Cluster F      | WS auth, MCP auth (currently anonymous)                    | Auth is its own cluster.                                           |
| Cluster H      | MCP stdio transport, SSE `resources/subscribe`             | Desktop / reactive consumers come with the web UI cluster.         |
| Cluster H      | Graceful shutdown, request-id middleware                   | Production polish cluster.                                         |
| Cluster U      | 1000-event WS soak, mutation tests against bus + routes    | Perf suite arrives in U.                                           |
| Cluster V      | SBOM (`cargo-cyclonedx`), Sigstore signing                 | Release polish cluster.                                            |

## Surprises

- **axum 0.7 vs 0.8 path syntax**: PR B.2 was written with `{id}`
  (0.8) before discovering the workspace pins 0.7 (`:id`). Cost: one
  test run + a `sed`. The fallback's silent 404 made the cause
  non-obvious.
- **axum 0.7 WebSocket types**: `CloseFrame` uses `Cow<'static, str>`
  for the reason and `Message::Text(String)` — not `Utf8Bytes`. That
  type lives in axum 0.8. Caught at compile time.
- **MCP tool envelope**: tool results are an array of typed content
  parts (`content: [{"type": "text", "text": "..."}]`), not a bare
  result object. The integration test had to unwrap the JSON string
  inside the first part.
- **cargo-deny on workspace path deps**: `wildcards = "deny"` flags
  workspace member path deps unless either `allow-wildcard-paths =
  true` is set AND every member crate is marked `publish = false`
  (path deps are forbidden on crates.io-published crates). Both
  changes were needed; only one was obvious.
- **Postgres NOTIFY 8 KB cap**: forced a small but real architectural
  decision. We pick "full JSON payload with size guard" for now;
  Cluster T can add an events table if 8 KB becomes a regular limit.
- **trufflehog action `--no-update` flag**: the action adds it itself,
  so passing it in `extra_args` produces `flag cannot be repeated`.
  Caught by the first PR-#20 CI run.

## Decisions

- **Filter logic stays subscriber-side for both bus backends.** Both
  `InMemoryBus` and `PostgresBus` broadcast every event to every
  subscriber; the filter runs in the subscriber's stream adapter.
  This keeps wire semantics identical across backends and means no
  backend-specific routing tables.
- **Bus publish failures are logged, not returned to HTTP callers.**
  The store has already committed; a temporarily-unavailable bus
  should not turn a successful mutation into a 5xx. Same pattern
  every emitter uses.
- **MCP transport is decoupled from the dispatcher.** `McpServer`
  takes a parsed request and returns a response. The HTTP wrapper
  is 8 lines; a future stdio loop will be similar.
- **Required-status-checks were applied AFTER PR #20 merged**, not
  before. Adding the checks before any green run would have made the
  bootstrap PR un-mergeable. Applied via `gh api PUT
  /branches/main/protection` immediately after the first green merge.
- **Two backends are tested with one shared suite.** `tests/common/mod.rs`
  (from Cluster A) extended to the HTTP CRUD test, the event-emission
  test, the WS test, and the MCP test — all run against SQLite by
  default and Postgres opportunistically where Docker is needed.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| GitHub Actions CI (lint + secrets + test + integration + e2e) | `v0.1.0`     |
| HTTP CRUD for workspaces / members / channels / threads / messages | `v0.1.0` |
| RFC 7807 `application/problem+json` errors              | `v0.1.0`           |
| Event taxonomy + filter (`EventFilter`)                 | `v0.1.0`           |
| `InMemoryBus` (tokio broadcast)                         | `v0.1.0`           |
| `PostgresBus` (LISTEN/NOTIFY)                           | `v0.1.0`           |
| WebSocket `/ws/subscribe` with filters                  | `v0.1.0`           |
| MCP `POST /mcp` (initialize + tools + resources)        | `v0.1.0`           |
| 7 MCP tools backed by the Store                         | `v0.1.0`           |
| Multi-arch ghcr.io image publish on tag                 | `v0.1.0`           |

See [[Capabilities]].

## Risks identified + mitigated

- **HTTP mutations and bus publish can drift** — if a mutation
  succeeds but publish fails silently, downstream consumers miss the
  event. Mitigated by logging publish failures with the event payload
  and the warning in [[Architecture]] that bus delivery is at-most-
  once until the event log lands.
- **NOTIFY payload exceeds 8 KB on large messages** — caught at the
  publish call with `BusError::PayloadTooLarge`. Documented; upgrade
  path is the persistent event log.
- **Slow WS subscribers stall publishers** — bounded mpsc + bus task
  abort on overflow. Verified by the WS handler's design (test for
  the actual overflow path is deferred to Cluster U).
- **MCP tools could exfiltrate cross-workspace data** — every tool
  takes a workspace-or-channel-or-thread id from the request, never
  from server state, so an MCP caller can only act on entities they
  already know an id for. Real authz lands in Cluster F.

## Risks identified + still open

- **Bus is at-most-once.** No retention, no replay. Subscribers that
  miss a notification have no recovery path. Persistent event log
  arrives in Cluster D.
- **No coverage gate yet.** CI builds and tests but doesn't enforce
  ≥ N% line/branch coverage. Cluster T adds it.
- **WS + MCP are anonymous.** Anyone with network access can subscribe
  / call tools. Cluster F (auth) is the gate.
- **Postgres bus background task lacks supervision.** If the listener
  errors permanently, it loops on a 1-second sleep but never reports
  upward; long-lived servers should surface that via the `/health`
  endpoint. Cluster T.

## Forward look

Cluster C is the next delivery cluster: search + indexing. Top
priorities at kickoff:

1. Full-text search index over messages (Postgres `tsvector`, SQLite
   FTS5).
2. Vector index for embeddings (`pgvector` is already bundled in the
   custom Postgres image).
3. Search routes (HTTP + MCP) that take a query and return ranked
   results with snippet highlights.
4. Background indexer that consumes the bus and keeps the indexes
   current.

See [[Roadmap]] for the full ladder.

## Acknowledgements

Solo cluster. The pi-rooted retro discipline pulled forward from
Cluster A continues to pay off — every PR's in-body retro became a
node in this aggregate retro, no archeology required.
