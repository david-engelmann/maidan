# Claims & evidence

Every load-bearing claim in the README and on the site should map to one of three
things: a **gate** (a tagged, CI-guarded milestone), a **test** (a named test or CI job
you can read), or an honest **"not yet."** This page is that map. If a sentence in our
marketing can't point at a row here, it shouldn't ship.

Maidan is **pre-1.0 and solo-maintained.** The tags are the engineering record; there is
no marketing "1.0." This page is kept honest by hand — if you find a claim that outruns
its evidence, that's a bug: open an issue.

## Product gates (tagged, CI-guarded)

| Gate | Tag | What it certifies |
|------|-----|-------------------|
| `maidan-2.0` | `v58.0.0` | Core collaboration surface |
| `maidan-agent-1.0` | `v76.0.0` | Agent-facing surface (MCP tools, subscribe) |
| `maidan-operator-1.0` | `v101.0.0` | Operator surface (audit, deliveries, reindex) |
| `maidan-scale-1.0` | `v120.0.0` | Scale-out (multi-replica, sharded fan-out, SLOs) |

All four gate tags are cut. Post-120 work ships on the same `vX.0.0` ladder as
post-gate hardening (no new gate tag).

## Claims → evidence

| Claim (README / site) | Evidence | Status |
|-----------------------|----------|--------|
| "Durable, shared memory: threads, results, artifacts, tool-call transcripts, all searchable" | `maidan-store` (Postgres + SQLite `Store` parity, `backend_parity` test); content-addressed artifacts; `thread_results`; `tool_transcript`; full-text (`tsvector`/FTS5) + semantic (`pgvector`) search | Shipped |
| "Tasks with dependencies, skill-based claiming, assignment + leases, scheduled runs, blocking waits" | Task-DAG + queue, scheduled/recurring tasks, skill routing, coordination waits (`wait_for_ready`/`wait_for_result`) — store tests `thread_deps`, `skill_routing`, `task_schedules`, `run_ready_dependents_suite`; e2es `thread_dependencies_e2e`, `thread_result_e2e` | Shipped |
| "Pull exactly the context a step needs — far fewer tokens" | Thread/workspace context packs (lean edits by default, `include_edits` opt-in); `snippet_only` search; capability-filtered `tools/list`; opt-in lean event frames; omit-empty metadata. **Measured: a scoped pack is ~6.8× fewer tokens than dumping the whole channel** (`token_pack` harness → [Benchmark.md](Benchmark.md#context-pack-token-savings-token_pack)) | Shipped + measured |
| "Access is scoped on every token; private channels enforced on reads, events, and search; every action is audited" | Capability model (every route + tool checks caps); per-channel/thread RBAC — e2es `channel_access_e2e`, `dm_participation_e2e`; filtered-ANN search excludes private channels in-query; subscribe-grant enforcement; audit trail — `audit_coverage_e2e` | Shipped |
| "Speaks MCP, REST, and WebSocket over one data model and one login" | One `AppState`/`Store`; REST (OpenAPI 3.0, `openapi_e2e` bijection), MCP (JSON-RPC + streamable HTTP), WebSocket subscribe — all bearer-authed | Shipped |
| "MCP-native — an MCP client connects directly and gets typed tools + live notifications" | `POST /mcp` + streamable HTTP; MCP `2026-07-28` (negotiated, default) with `2024-11-05` fallback; `resources/updated`; live-verified LangChain + AutoGen recipes (`docs/Framework Integrations.md`) | Shipped |
| "Single static binary, laptop SQLite → multi-replica Postgres cluster" | One binary selected by `DATABASE_URL`; `scale-out smoke` required CI job; workspace-sharded fan-out; LSN causal read-replica routing (`read_routing` e2e vs real streaming replication) | Shipped (`maidan-scale-1.0`) |
| "Operationally honest — probes, Prometheus, OTLP, durable event log + replay, cross-replica correctness" | `/health/{live,ready}`; `/metrics`; `otlp smoke` + `promtool (alert rules)` required CI; transactional outbox (events commit atomically with their domain write); self-healing NOTIFY floor (chaos-validated 40/40) | Shipped |
| "Signed release artifacts" | Keyless cosign bundles + SBOM on every release (`release.yml`); per-arch tarballs SHA-256-pinned in the quickstart image. Verify: see [SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release) | Shipped |
| "A2A transport" | A2A v1.0 over **JSON-RPC + REST §11** (complete); **gRPC §10 exposes task read/cancel/list** (`get_task`/`cancel_task`/`list_tasks`) — **`SendMessage`/push/streaming over gRPC are not yet implemented; send a message over JSON-RPC or REST**. Agent Card §4.4.1; interop conformance client + report-only `a2a interop` CI job | Shipped (JSON-RPC/REST complete; gRPC partial) |
| "Off-platform reach: notifications, email, Slack, GitHub" | Per-recipient notification ledger + router + unified inbox; SMTP transport + durable mail retry queue (outbox + worker + DLQ); Slack + GitHub projectors (bidirectional, loop-safe) | **Shipped, config-gated** — inert until you set `MAIDAN_SMTP_*` / `MAIDAN_SLACK_*` / `MAIDAN_GITHUB_*` and create the apps |
| "Client SDKs" | Four 0.1.0 clients (TypeScript, Python, Go, Rust) to the frozen v1 contract, each black-box-verified (`scripts/sdk-test.sh`) + a report-only `sdk interop` CI job | Shipped (0.1.0, early) |

## Not yet / honest limits

- **No hosted SaaS.** Maidan is self-hosted only. There is no managed cloud and no
  public playground.
- **Not a library on crates.io.** The workspace is `publish = false` on purpose; the
  "release" is the tagged binary + container image, not a crate.
- **Projectors + email are config-gated and unproven in public.** The code ships and is
  tested with mocks; a live Slack/GitHub/SMTP deployment needs you to create the app and
  set the secrets. We don't claim a running public instance.
- **OIDC human login is present but maturing.** `MAIDAN_OIDC_*` enables `/auth/oidc/*` +
  session mint; treat it as config-gated, not a polished consumer login.
- **SDKs are 0.1.0.** Usable against the frozen v1 contract, dependency-light, but early
  — typed response models and registry-published interop CI are follow-ups.
- **Not an orchestration planner or an agent runtime.** Maidan does not run your models
  or decide how an agent reasons. It is the durable place agents coordinate.

## How this stays honest

- New public claims add a row here in the same PR.
- The required CI checks (lint, secrets scan, unit, integration, docker-compose smoke,
  scale-out smoke, promtool, otlp smoke) gate every merge; the report-only jobs (`a2a
  interop`, `sdk interop`) prove the client/interop surface without blocking.
- Release artifacts are cosign-signed; verify before trusting a tag
  ([SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release)).
