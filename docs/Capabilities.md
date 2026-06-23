# Capabilities

A running list of what Maidan can do, by release. Each cluster's retro
PR prepends a new section so the latest is always at the top.

## v123.0.0 — OTLP delivery proven end-to-end

| Capability | Where |
|------------|-------|
| OTLP traces + metrics asserted against a real collector in CI | `compose.yaml` (`otlp` profile), `docker/otel-collector-config.yaml`, `scripts/otlp-smoke.sh`, `.github/workflows/ci.yml` (`otlp smoke`) |

_Post-gate hardening (Phase XXIV): closes the residual observability gap from Cluster 122 — the OTLP export wiring (Cluster 89) is now proven against a running collector, not just an in-process unit test. No new gate tag._

## v122.0.0 — Alert rules executed in CI

| Capability | Where |
|------------|-------|
| SLO recording/alert PromQL executed in CI (`check rules` + unit tests) | `.github/workflows/ci.yml` (`promtool (alert rules)`), `scripts/check-alert-rules.sh` |
| SLO rule unit tests (queue-sat guard, embed-failure restart-safety, `$value`) | `docs/alerts/prometheus-rules-maidan-slo.test.yaml` |

_Post-gate hardening (Phase XXIV): closes the "alert exprs never executed" gap from Cluster 121 — which immediately caught a `$value`-rendering bug in `MaidanIndexerQueueSaturated`. Also corrects the OTLP-export status (shipped in Cluster 89). No new gate tag._

## v121.0.0 — Observability & contract completeness

| Capability | Where |
|------------|-------|
| Every OpenAPI op classified (bearer / session / public) in CI | `crates/maidan-server/tests/http_openapi_capability_map_contract.rs` |
| Indexer queue-saturation recording rule + backpressure/embed-failure alerts | `docs/alerts/prometheus-rules-maidan-slo.yaml` |
| Operator dashboard panels for indexer queue depth + embed failures | `docs/dashboards/maidan-operator.json` |

_Post-gate hardening (Phase XXIV): closes the OpenAPI-wide capability-map gap (Cluster 69) and extends the Cluster 90 SLO surface to the Cluster 116 indexer metrics. No new gate tag._

## v120.0.0 — Scale product gate (`maidan-scale-1.0`)

| Capability | Where |
|------------|-------|
| `maidan-scale-1.0` gate (criteria → evidence) | `docs/Gates/maidan-scale-1.0.md`, `maidan_scale_gate_e2e` |
| Recorded store bench baseline | `crates/maidan-store/benches/STORE_BASELINE.md` |
| `scale-out smoke` as a gate-required check | `.github/workflows/ci.yml` |

_Closes Product Ladder 102+ (gate **`maidan-scale-1.0`** at **`v120.0.0`**)._

## v119.0.0 — Dependency dedupe & currency

| Capability | Where |
|------------|-------|
| Duplicate-major CI gate (`multiple-versions = deny`) | `deny.toml` (`lint` job) |
| Dependency currency + duplicate-version policy doc | `docs/Dependencies.md` |
| Workspace on thiserror 2 | `Cargo.toml` |

## v118.0.0 — Hybrid relevance

| Capability | Where |
|------------|-------|
| Hybrid lexical+semantic search (HTTP + MCP) | `crates/maidan-server/src/routes/search.rs`, `crates/maidan-mcp/src/tools/search.rs` |
| Score fusion (`fuse_hybrid`, `DEFAULT_HYBRID_WEIGHT`) | `crates/maidan-search/src/score.rs`, `traits.rs` |
| Relevance eval harness | `crates/maidan-search/tests/relevance_eval.rs` |

## v117.0.0 — Pluggable production provider

| Capability | Where |
|------------|-------|
| Production `openai-compatible` embeddings with auto-detected dimension | `crates/maidan-search/src/embedding_provider.rs` |
| Boot-time per-model registration (`Search::ensure_model`) | `crates/maidan-search/src/traits.rs`, `postgres.rs`, `sqlite.rs` |
| Embedding provider + model-migration guide | `docs/Embeddings.md` |

## v116.0.0 — Batch embedding pipeline

| Capability | Where |
|------------|-------|
| Batched live embedding indexer (bounded queue + backpressure) | `crates/maidan-search/src/embedding_batcher.rs` |
| Batch embedding provider API (`embed_batch`) | `crates/maidan-search/src/embedding_provider.rs` |
| Chunked large-workspace backfill | `crates/maidan-search/src/reindex.rs` |
| Bounded indexer-lag + throughput metrics | `crates/maidan-server/src/metrics.rs` (`maidan_indexer_queue_depth`, …) |

## v115.0.0 — Module split + `unwrap()` purge

| Capability | Where |
|------------|-------|
| No non-test `unwrap()`/`expect()` in `crates/*/src` (clippy-enforced) | `.github/workflows/ci.yml` (lint job) |
| Domain-organized HTTP route modules | `crates/maidan-server/src/routes/` |
| Domain-organized MCP tool modules | `crates/maidan-mcp/src/tools/` |

## v114.0.0 — Coverage uplift + envelope fuzz

| Capability | Where |
|------------|-------|
| Full-suite coverage gate (≥ 40% lines) | `.github/workflows/ci.yml` (`coverage` job) |
| JSON-RPC / MCP / A2A envelope round-trip + fuzz coverage | `maidan-mcp/src/{protocol,error}.rs`, `maidan-a2a/src/protocol.rs` |

## v113.0.0 — Backend parity harness

| Capability | Where |
|------------|-------|
| Migration + store-module lockstep guard (allowlisted) | `maidan-store/tests/backend_parity.rs` |
| Cross-dialect identity over FSM / edit / reaction surface | `maidan-store/tests/{common/mod.rs,dialect_parity.rs}` |

## v112.0.0 — FSM property tests

| Capability | Where |
|------------|-------|
| FSM transition + rank invariants under arbitrary inputs | `maidan-fsm/tests/fsm_properties.rs` |
| Hierarchical (tree-wide) rank-rule guarantee | `maidan-fsm/tests/fsm_properties.rs` (`locally_valid_tree_is_globally_consistent`) |

## v111.0.0 — `maidan-auth` test suite

| Capability | Where |
|------------|-------|
| Capability-vocabulary + `AuthContext` authorization matrix coverage | `maidan-auth/tests/capability_matrix.rs` |
| Peer-secret AEAD round-trip / tamper / key-parse coverage | `maidan-auth/tests/peer_secret_aead.rs` |
| Bearer lifecycle (mint / revoke / expire / forge) coverage | `maidan-auth/tests/token_lifecycle.rs` |

## v110.0.0 — Per-workspace fairness

| Capability | Where |
|------------|-------|
| Per-workspace request-rate fairness | `rate_limit::middleware`, `MAIDAN_WORKSPACE_RATE_LIMIT_MAX` (key `ws:{wid}`) |
| Noisy-neighbor regression guard | `tenant_fairness_e2e` |

## v109.0.0 — ANN index tuning + search bench

| Capability | Where |
|------------|-------|
| Tunable HNSW build + query params | `hnsw::HnswParams`, `ensure_model_postgres`, `PostgresSearch::semantic_search` |
| Lexical + semantic latency bench + baseline | `maidan-search/benches/search_hot.rs`, `SEARCH_BASELINE.md` |

## v108.0.0 — Adaptive outbox relay

| Capability | Where |
|------------|-------|
| Drain-until-empty + idle backoff relay cadence | `OutboxRelay::run`, `RelayTick`, `backoff_step` |
| Prompt wake on enqueue (polling-safe mpsc nudge) | `AppState.outbox_nudge`, `OutboxRelay::with_nudge`, `wait_idle_or_nudge` |

## v107.0.0 — Configurable DB pool & timeouts

| Capability | Where |
|------------|-------|
| Env-tunable pool size + acquire timeout | `config::DbConfig`, `main.rs` |
| Postgres `statement_timeout` (migration-exempt) / SQLite `busy_timeout` | `after_connect` cap, `configure_sqlite_pool_with` |

## v106.0.0 — Bulk context reads

| Capability | Where |
|------------|-------|
| O(1)-query context assembly (no per-row N+1) | `thread_context.rs`, `Store::{list_threads_for_workspace, list_references_from_many, list_message_edits_for_messages}` |
| Query-count regression guard | `context_query_count_e2e` |

## v105.0.0 — Multi-replica scale-out smoke

| Capability | Where |
|------------|-------|
| Race-free boot migrations under N replicas | `run_postgres_migrations` advisory lock, `concurrent_migrations` test |
| Tested two-replica topology (shared PG + object store + LB) | `compose.yaml` `scale` profile, `scripts/scale-out-smoke.sh`, CI `scale-out smoke` |

## v104.0.0 — Durable ephemeral state

| Capability | Where |
|------------|-------|
| Durable single-use OAuth codes (any-replica exchange) | `maidan_oauth_codes`, `Store::{insert,consume}_oauth_code`, `app_oauth.rs` |
| Durable reindex job status (any-replica read) | `maidan_reindex_jobs`, `Store::{upsert,get}_reindex_job`, `reindex_ops.rs` |

## v103.0.0 — Distributed presence & roster

| Capability | Where |
|------------|-------|
| Cross-replica presence/typing fan-out | `maidan-bus::PresenceNotifier`, `PostgresPresenceNotifier` (`maidan_presence`) |
| Merged TTL roster across replicas | `PresenceHub` heartbeat + sweep, `AppState::attach_presence_notifier` |

## v102.0.0 — Cross-replica MCP resource notifications

| Capability | Where |
|------------|-------|
| Cross-process resource-update fan-out | `maidan-bus::ResourceNotifier`, `PostgresResourceNotifier` (`maidan_resource_updated`) |
| Per-replica notification delivery | `McpServer::spawn_resource_notify_listener`, `AppState::attach_resource_notifier` |

## v101.0.0 — Operator product gate

| Capability | Where |
|------------|-------|
| Operator gate e2e | `maidan_operator_gate_e2e.rs` |

## v100.0.0 — mcp-stdio embedded indexer

| Capability | Where |
|------------|-------|
| Stdio + in-process indexer | `maidan-cli` `mcp-stdio`, `McpServer::with_event_bus` |

## v99.0.0 — Presence v2 docs

| Capability | Where |
|------------|-------|
| Roster + WS presence guide | `docs/Presence and Roster.md` |

## v98.0.0 — Mention webhook router

| Capability | Where |
|------------|-------|
| Workspace mention webhook config | `mention_webhook_id`, `webhooks.rs` |

## v97.0.0 — Group DMs

| Capability | Where |
|------------|-------|
| Group DM (≥3 members) | migrations 0027/0028, `group_dm.rs` |

## v96.0.0 — /ui tokens & apps

| Capability | Where |
|------------|-------|
| List API tokens | `GET .../members/:mid/tokens` |
| UI token + app install list | `static/index.html` |

## v95.0.0 — /ui search

| Capability | Where |
|------------|-------|
| Faceted search tab | `/ui` search panel + `/ui/api/.../search` |

## v94.0.0 — /ui artifacts

| Capability | Where |
|------------|-------|
| Artifact cards + attach | `renderMessages`, upload flow |

## v93.0.0 — /ui live events

| Capability | Where |
|------------|-------|
| WS presets + reconnect + session subscribe | `index.html`, `ws.rs` |
| E2e | `ui_ws_tail_e2e.rs` |

## v92.0.0 — /ui channel browser

| Capability | Where |
|------------|-------|
| Session cookie writes on `/ui/api` | `POST` channels, threads, messages |
| Channel browser in static UI | `static/index.html` (`data-ui-version="6"`) |
| E2e | `ui_channels_e2e.rs` |

## v88.0.0 — Helm production profiles

| Capability | Where |
|------------|-------|
| OTel / Redis / S3 values overlays | `helm/maidan/values-profile-*.yaml` |
| Profile install guide | `helm/maidan/PROFILES.md` |
| Profile helm template smoke | `scripts/helm-template-smoke.sh` |

## v90.0.0 — SLO alert templates

| Capability | Where |
|------------|-------|
| Prometheus SLO rules + Alertmanager example | `docs/alerts/` |
| Rules validation script | `scripts/validate-prometheus-rules.sh` |
| Alert/metric contract test | `maidan-server/tests/alert_templates_contract.rs` |

## v89.0.0 — OTLP metrics export

| Capability | Where |
|------------|-------|
| OTLP metrics push (fanout with Prometheus) | `OTLP_METRICS`, `maidan-server::metrics`, `maidan-observability::metrics` |
| Example Grafana dashboard | `docs/dashboards/maidan-operator.json` |
| Helm otel profile enables metrics | `values-profile-otel.yaml` |

## v87.0.0 — Reindex job API

| Capability | Where |
|------------|-------|
| Operator reindex enqueue + poll | `POST/GET /operator/reindex-embeddings` |
| `Search::reindex_embeddings` | `maidan-search` Postgres + SQLite |
| Reindex job e2e | `maidan-server/tests/reindex_job_e2e.rs` |

## v86.0.0 — Per-model embedding query

| Capability | Where |
|------------|-------|
| `embedding_model` search param | `SearchQuery`, MCP `search_messages`, [[Production]] |
| Model-scoped semantic HTTP e2e | `search_semantic_e2e.rs` |

## v85.0.0 — sqlite-vec optional

| Capability | Where |
|------------|-------|
| Optional `sqlite-vec` feature | `maidan-search/Cargo.toml`, `maidan-server` feature `sqlite-vec` |
| CI linkage proof | `.github/workflows/ci.yml` job `sqlite-vec (optional feature)` |
| Brute-force SQLite semantic (default) | `SqliteSearch::semantic_search` without feature |

## v84.0.0 — Outbox relay modes

| Capability | Where |
|------------|-------|
| Polled outbox relay | `MAIDAN_OUTBOX_RELAY_MODE=polled`, `PostgresBusOptions` |
| Production outbox guard | `validate_startup` in `outbox_relay`, `MAIDAN_ENV=production` |
| SQLite outbox on by default | `main.rs` sqlite dialect |

## v83.0.0 — SQLite delivery cursor (ladder close)

| Capability | Where |
|------------|-------|
| SQLite delivery cursor | `maidan_delivery_cursor` migration `0023`, `SqliteStore::get/advance_delivery_cursor` |
| Cursor parity tests | `maidan-store/tests/delivery_cursor.rs` |

## v82.0.0 — Context pagination

| Capability | Where |
|------------|-------|
| Paginated thread context | `GET /threads/:id/context` (`message_cursor`, `next_message_cursor`) |
| Paginated workspace context | `GET /workspaces/:id/context` (`thread_cursor`, `next_thread_cursor`) |
| MCP context cursors | `get_thread_context` / `get_workspace_context` tool args |

## v81.0.0 — Subscribe grants v3

| Capability | Where |
|------------|-------|
| WS `channel_grants` | Subscribe frame filter; schema v3 |
| Private channel enforcement | `subscribe_grants`, `EventFilter::matches` |
| MCP stream grants | `GET /mcp/stream?channel_grants=…` |

## v79.0.0 — A2A long-running tasks

| Capability | Where |
|------------|-------|
| Task cancel | `tasks/cancel` on `POST /a2a/v1/rpc` |
| Subscribe progress | `SubscribeToTask` `statusUpdate` SSE frames |
| Terminal subscribe guard | JSON-RPC `-32005` |

## v80.0.0 — Delivery ops unified

| Capability | Where |
|------------|-------|
| Unified delivery list/get/replay | `GET/POST /workspaces/:wid/deliveries` |
| Webhook delivery operator store API | `list_webhook_deliveries`, `replay_webhook_delivery` |
| Automation routes (legacy) | `/workspaces/:wid/automation/deliveries` |

## v77.0.0 — HTTP capability map complete

| Capability | Where |
|------------|-------|
| Full HTTP capability map | `contracts/http-capability-map.json` |
| OpenAPI ↔ map CI | `http_openapi_capability_map_contract.rs` |
| HTTP deny matrix e2e | `http_capability_matrix_e2e.rs` |
| OpenAPI route parity | `openapi/paths/extensions.rs`, multipart stubs |

## v76.0.0 — Agent observability (`maidan-agent-1.0`)

| Capability | Where |
|------------|-------|
| Agent substrate gate e2e | `agent_substrate_gate_e2e.rs` |
| Ops runbook | [[Production#Agent observability]] |

## v72.0.0 — A2A task streaming

| Capability | Where |
|------------|-------|
| Persisted push config | `maidan_a2a_push_configs` |
| Persisted tasks | `maidan_a2a_tasks` |
| SubscribeToTask SSE | `POST /a2a/v1/rpc` |
| Push on task update | Best-effort POST to configured URL |

## v74.0.0 — MCP context export

| Capability | Where |
|------------|-------|
| `get_thread_context` | MCP `tools/call` |
| `get_workspace_context` | MCP `tools/call` |

## v71.0.0 — Subscribe contract v2

| Capability | Where |
|------------|-------|
| WS filter schema | `contracts/ws-subscribe-filter.schema.json` |
| EventKind forward-compat | [[Agent Integration]] |

## v70.0.0 — Vault truth pass

| Capability | Where |
|------------|-------|
| Architecture snapshot `v69` | [[Architecture]] |
| Reconciled backlog docs | [[Remaining Work]], [[Open Work]] |
| Agent integration README pitch | Root `README.md`, [[Agent Integration]] |

## v69.0.0 — Capabilities matrix complete

| Capability | Where |
|------------|-------|
| MCP tool → capability map | `contracts/mcp-capability-map.json` |
| MCP matrix e2e | `mcp_capability_matrix_e2e.rs` |
| HTTP capability contract | `contracts/http-capability-routes.json` |
| Contract CI | `scripts/check-agent-contract.sh` |

## v68.0.0 — Automation delivery guarantees

| Capability | Where |
|------------|-------|
| Automation delivery ledger | `maidan_automation_deliveries` (slash + FSM HTTP) |
| Retry worker | `maidan-server::automation_worker` |
| List / replay / DLQ | `GET/POST /workspaces/:wid/automation/*` |
| Slash sync-then-queue | `maidan-server::slash_commands` |
| FSM async HTTP dispatch | `maidan-server::fsm_hooks` |

## v67.0.0 — Workspace context packages

| Capability | Where |
|------------|-------|
| Workspace context export | `GET /workspaces/:id/context` |
| Message edits in thread context | `GET /threads/:id/context` |

## v65.0.0 — App install OAuth

| Capability | Where |
|------------|-------|
| OAuth authorization code | `POST .../apps/:app_id/oauth/authorize` |
| Token exchange | `POST /oauth/app/token` |

## v62.0.0 — Subscribe schema + outbox list

| Capability | Where |
|------------|-------|
| WS subscribe schema version | `subscribe_ack.schema_version` |
| List quarantined outbox | `GET /workspaces/:wid/outbox/quarantined` |

## v60.0.0 — MCP streamable session lifecycle

| Capability | Where |
|------------|-------|
| Streamable session TTL | `MAIDAN_MCP_STREAMABLE_SESSION_TTL_SECS` |
| Close streamable session | `DELETE /mcp/streamable` |

## v59.0.0 — Agent integration charter

| Capability | Where |
|------------|-------|
| Agent integration guide | [[Agent Integration]] |
| Event/tool contract CI | `scripts/check-agent-contract.sh` |

## Maidan 2.0 product gate (`maidan-2.0`)

| Capability | Where |
|------------|-------|
| Product Ladder 35–58 closed | [[Retros/Product Ladder 35+]] |
| Checklist sign-off | [[Product Completion Checklist]] at **`v58.0.0`** |

## v58.0.0 — Maidan 2.0 completion gate

| Capability | Where |
|------------|-------|
| Product completion checklist (28–57) | [[Product Completion Checklist]] |
| Expanded completion gate e2e | `product_completion_gate_e2e.rs` |

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
