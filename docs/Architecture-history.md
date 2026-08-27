# Architecture history

How Maidan's architecture accrued, release by release. The current, version-neutral
shape lives in [Architecture.md](Architecture.md); this file is the historical record —
useful for understanding *when* and *why* a subsystem took its shape.
[Capabilities.md](Capabilities.md) and [CHANGELOG.md](../CHANGELOG.md) are the
authoritative release lists.

## Agent substrate snapshot (`v67`–`v76`)

| Area | Shipped | Notes |
|------|---------|-------|
| **Discovery** | `/.well-known/maidan.json`, agent card | MCP + A2A entry points |
| **MCP tools** | tools in `contracts/mcp-tool-names.json` | Per-tool caps in `contracts/mcp-capability-map.json`; CI matrix (**69**) |
| **MCP streamable** | `POST/DELETE /mcp/streamable`, mux on SSE (**78**, **73**) | Subset of 2024-11-05; see [Integration.md](Integration.md) |
| **Subscribe** | WS filter schema + MCP SSE, `channel_grants` (**81**, **71**) | |
| **A2A** | RPC + `SubscribeToTask` + cancel/progress (**72**, **79**) | Full v1.0 multi-transport later (282–289) |
| **Apps** | Installed apps + OAuth code exchange (**57**, **65**) | App-scoped bearer secrets |
| **Quotas** | Per-token capability quotas on MCP `tools/call` (**64**) | Redis optional for distributed windows (**54**) |
| **Automation** | Webhooks (**50**), slash (**51**), FSM hooks (**52**) | Slash/FSM on `maidan_automation_deliveries` + DLQ (**68**) |
| **Context** | HTTP + MCP `get_*_context` with cursors (**74**, **82**) | |
| **Privacy** | Message purge, deep workspace erase (**53**), audit | |
| **Deploy** | `helm/maidan`, `helm/maidan-stack`, cert-manager values (**55**), profile overlays (**88**) | Bootstrap compile-time strip (**91**) |
| **Product gates** | **`maidan-2.0`** **`v58`** · **`maidan-agent-1.0`** **`v76`** · **`maidan-operator-1.0`** **`v101`** · **`maidan-scale-1.0`** **`v120`** | Ladder **77–101** ([Clusters/Product Ladder 77+.md](Clusters/Product%20Ladder%2077+.md)) and **102–120** ([Clusters/Product Ladder 102+.md](Clusters/Product%20Ladder%20102+.md)); scale gate [Gates/maidan-scale-1.0.md](Gates/maidan-scale-1.0.md) |

## Artifacts at v0.4.0

- **Kinds** — `screenshot`, `recording`, `transcript`, `code_dump`, `attachment`
  (`ArtifactKind` + DB CHECK).
- **Storage** — content-addressed fanout keys in LocalFs and S3.
- **HTTP** — `POST /artifacts?kind=…` stores body then upserts metadata; publishes
  `ArtifactUpserted`.
- **MCP** — `upload_artifact` (base64), `get_artifact_metadata`,
  `maidan://artifacts/{sha256}` resource. Per-workspace access refs added later (Cluster 204).

## Thread lifecycle at v0.4.0

- **States** — `open` → `in_review` → `closed` → `archived` on `maidan_threads.state`.
- **FSM** — `maidan-fsm::apply` validates edges; illegal transitions return 409.
- **Transition log** — `maidan_thread_transitions` records every transition.
- **Nested threads** — `parent_thread_id`; HSM ensures a child's lifecycle rank does not
  outrun its parent.
- **Events** — `ThreadStateChanged` on the bus when a transition commits.

(The assignment/claim/lease axis, dependency DAG, skill routing, scheduling, and
structured results were layered on this in the agentic arcs — Clusters 171, 190–236.)

## Search at v1.2.0 → v5.0.0

- **Lexical** — Postgres `tsvector` + GIN with `ts_headline`; SQLite FTS5 + `snippet()`;
  `websearch_to_tsquery` for web-style operators. Facets: `author`, `channel`, author
  `kind`.
- **Semantic** — Postgres `pgvector` `vector(1024)` + HNSW cosine (`v1.3.0`); facets since
  `v3.0.0`. SQLite semantic added at `v18.0.0`.
- **Rank vs score** — `rank` is backend-specific "higher is better"; **`score`** is
  normalized to `[0,1]` within a response and comparable across backends (`v48.0.0`).
- **Indexer** — `maidan-search::Indexer` on `MessagePosted`/`MessageTombstoned`, pluggable
  `EmbeddingProvider`.

## Auth at v0.5.0

- **API tokens** — SHA-256 hashed secrets in `maidan_api_tokens`; capabilities as JSON;
  optional expiry + revocation.
- **OIDC + sessions (`v2.0.0`)** — authorization code + PKCE; `maidan_session` cookie;
  first `token:admin` via `POST /auth/session/mint`.
- **WebSocket** — `SubscribeFrame.token`; requires `event:subscribe`. `v4.0.0` adds HMAC
  `resume_token` + `replay_truncated`.
- (Per-channel/thread RBAC across all surfaces landed in Clusters 159–165 + 179–204.)

## Subscriber continuity at v4.0.0

Replay-from-watermark on reconnect (up to 500 rows, `replay_truncated` beyond),
`subscribe_ack` with a signed `resume_token`, live events after, and auto-replay on bus
lag when a workspace filter is set.

## Delivery reliability at v6.0.0

Prometheus series alongside `/health`: `maidan_bus_lag_total{transport}`,
`maidan_subscribe_replay_total{transport,outcome}`, `maidan_indexer_last_event_age_seconds`,
`maidan_bus_listener_ok` / `_errors_total`. Fixed label sets (no workspace-UUID labels).

## Bus pointer delivery at v7.0.0

On Postgres, `pg_notify` carries a small `{log_id}` pointer (not full event JSON) for the
normal path; the LISTEN task hydrates the envelope from `maidan_events`. Synthetic
publishes (`log_id == 0`) keep the legacy full-envelope NOTIFY.

## Bus hydrate observability at v8.0.0

The listener increments `maidan_bus_notify_hydrate_total{result}`
(`ok`/`not_found`/`failed`/`invalid_payload`) per hydrate attempt.

## Transactional outbox at v10.0.0 / v14.0.0

Event append + outbox enqueue share a transaction; a relay publishes after commit
(`maidan_outbox_pending`, `maidan_outbox_relay_total`). The full domain-write ⊗
event-append atomicity refactor completed across every mutation in Clusters 205–214.

## Outbox quarantine at v12.0.0

After `MAIDAN_OUTBOX_MAX_ATTEMPTS` failed publishes the relay sets `quarantined_at` and
stops selecting the row (pending → published | quarantined). Metrics:
`maidan_outbox_quarantined`, `maidan_outbox_oldest_pending_seconds`.

## Delivery cursors at v13.0.0

`maidan_delivery_cursor` tracks `last_delivered_log_id` per `(consumer_id, workspace_id)`;
WS + MCP SSE accept an optional `consumer_id`; monotonic advance (`GREATEST`); clients
treat `log_id` as idempotent under duplicate NOTIFY. This is the at-least-once path.

## At v0.6.0 (Cluster G) — federation

`maidan_peers` registry, `POST /a2a/v1/events` ingest, `FederationWorker` poll,
`maidan-a2a::Outbound`, `/.well-known/maidan.json`; peer bearer distinct from member
tokens (`federation:ingest`/`federation:admin`).

## At v0.7.0 (Cluster H)

Static `/ui/` event tail; `maidan mcp-stdio`; `GET /mcp/stream` SSE; graceful shutdown,
`X-Request-Id`, `/health/live` + `/health/ready`.

## At v15.0.0–v18.0.0 (MCP resources + SQLite semantic)

MCP `resources/subscribe`/`unsubscribe` + `notifications/resources/updated` (stdio then
HTTP via `GET /mcp/notifications`); resource fan-out to thread/channel/workspace/artifact
URIs; SQLite semantic search (`maidan_message_embeddings`, later per-model tables).

## At v23.0.0–v27.0.0 (Product Ladder close)

`/ui` tabs (events, search, thread FSM, token mint); `helm/maidan`; workspace purge;
MCP streamable HTTP subset (`POST /mcp/streamable`).

## Per-model embeddings at v47.0.0

The single `maidan_message_embeddings` table was replaced by a registry
(`maidan_embedding_models`) plus one vector table per model (e.g. `maidan_emb_hash_v1`).
Swapping/adding a provider no longer filters a shared table by `model`; each model is
isolated (clean reindex, no stale-vector cross-talk, per-model dimension). The reindex job
rebuilds a model's table from scratch.

## Cross-replica resource notifications at v102.0.0

`resources/updated` fan out across replicas: touched `maidan://` URIs are published
unfiltered on the `maidan_resource_updated` NOTIFY channel via
`maidan-bus::ResourceNotifier`; each replica applies its own local subscription filter and
delivers to its SSE subscribers. Single delivery path (originating replica also delivers
via its listener) — no de-duplication needed. SQLite/polled-relay use the in-memory
notifier.

## Distributed presence at v103.0.0

`maidan-bus::PresenceNotifier` (`maidan_presence` NOTIFY) carries typed `PresenceEvent`s;
each replica folds a merged, TTL-expiring remote view and fans frames to its WS
subscribers. Heartbeats refresh TTLs silently; `presence_snapshot` merges local +
non-expired remote. TTL/heartbeat env-tunable, receiver-stamped (skew-safe), gated to
Postgres+NOTIFY.

## Durable ephemeral state at v104.0.0

App OAuth authorization codes (`maidan_oauth_codes`, single-use atomic redeem) and reindex
job status (`maidan_reindex_jobs`) moved from process memory into the store, so they work
across replicas and survive restart.

## Scale-out & hardening (Ladder 102+)

Clusters 102–120 (`v102.0.0`–`v120.0.0`) hardened the substrate for multi-replica
operation and search-at-scale:

- **XIX — scale-out core (102–105):** `≥2` replicas behind a load balancer sharing one
  Postgres + object store; cross-replica resource notifications, presence/roster, and
  OAuth/notify-across-pods; `scale-out smoke` CI job.
- **XX — hot-path hardening (106–110):** bounded query counts (no N+1), configurable pool
  + outbox relay, ANN/HNSW tuning knobs, per-workspace fairness.
- **XXI — correctness & coverage (111–115):** a ≥40% coverage floor in CI; auth suite, FSM
  property tests, Postgres↔SQLite parity harness, envelope fuzz; no non-test
  `unwrap()/expect()` in `crates/*/src`; `routes.rs`/`tools.rs` split into modules.
- **XXII — search & indexer at scale (116–118):** bounded back-pressured embed queue;
  pluggable `openai-compatible` provider; hybrid search with a relevance eval harness.
- **XXIII — supply chain & scale gate (119–120):** thiserror 2; `cargo deny`
  `multiple-versions = "deny"`; the **`maidan-scale-1.0`** gate promotes `scale-out smoke`
  to a required check.

## Post-gate hardening (Phase XXIV, Cluster 121+)

Opportunistic hardening on the same `vX.0.0` ladder with no new gate tag. Highlights: OTLP
+ promtool CI (121–124); opt-in at-least-once delivery (125–126); delivery/MCP hardening
(128–132); the `/ui` collaboration surface (133–153); a security-led four-program run —
security round 2 incl. the transactional-outbox refactor (202–216), agentic orchestration
(task DAG/scheduling/skills/queue-depth/results, 217–236), notifications & reach
(237–257), and scale & durability incl. the LSN causal read-replica (258–266); launch
readiness (276–281); and the **A2A v1.0 multi-transport compliance arc** (282–289). See
[Capabilities.md](Capabilities.md) and [CHANGELOG.md](../CHANGELOG.md) for the full record.
