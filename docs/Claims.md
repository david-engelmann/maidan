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
| "Access is scoped on every token; private channels enforced on reads, events, and search; **privileged** actions are audited" | Capability model (every route + tool checks caps); per-channel/thread RBAC — e2es `channel_access_e2e`, `dm_participation_e2e`; filtered-ANN search excludes private channels in-query; subscribe-grant enforcement; every successful change attributed; 40 named privileged audit actions, authority changes written transactionally (fail-closed) — `audit_coverage_e2e` | Shipped — see the audit-scope note below |
| "Speaks MCP, REST, and WebSocket over one data model and one login" | One `AppState`/`Store`; REST (OpenAPI 3.1, `openapi_e2e` bijection), MCP (JSON-RPC + streamable HTTP), WebSocket subscribe — all bearer-authed | Shipped |
| "MCP-native — an MCP client connects directly and gets typed tools + live notifications" | `POST /mcp` + streamable HTTP; MCP `2026-07-28` (negotiated, default) with `2024-11-05` fallback; `resources/updated`; live-verified LangChain + AutoGen recipes (`docs/Framework Integrations.md`) | Shipped |
| "Single static binary, laptop SQLite → multi-replica Postgres cluster" | One binary selected by `DATABASE_URL`; `scale-out smoke` required CI job; workspace-sharded fan-out; LSN causal read-replica routing (`read_routing` e2e vs real streaming replication) | Shipped (`maidan-scale-1.0`) |
| "Built to be run, not just demoed — probes, Prometheus, OTLP, durable event log + replay, cross-replica correctness" | `/health/{live,ready}`; `/metrics`; `otlp smoke` + `promtool (alert rules)` required CI; transactional outbox (events commit atomically with their domain write); **leased outbox claim so N replicas relay each row once** (`concurrent_relays_claim_disjoint_outbox_rows`); self-healing NOTIFY floor (`notify_floor::sim` unit test, 400 seeds by default, replayed with `MAIDAN_SIM_SEED`) | Shipped |
| "Signed release artifacts" | Keyless cosign bundles + SBOM on every release (`release.yml`); per-arch tarballs SHA-256-pinned in the quickstart image. Verify: see [SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release) | Shipped |
| "A2A transport" | A2A v1.0 over **JSON-RPC, REST §11 and gRPC §10**, all complete. The gRPC binding (opt-in) serves the official `lf.a2a.v1.A2AService` from the unmodified v1.0.1 `a2a.proto`, every operation over the same handlers as the other two. Agent Card §4.4.1. The official A2A TCK runs over all three bindings in the non-required `a2a tck` CI job; exclusions are listed in `scripts/a2a-tck/exclusions.txt` | Shipped (all three bindings) |
| "Off-platform reach: notifications, email, Slack, GitHub" | Per-recipient notification ledger + router + unified inbox; SMTP transport + durable mail retry queue (outbox + worker + DLQ); Slack + GitHub projectors (bidirectional, loop-safe) | **Shipped, config-gated** — inert until you set `MAIDAN_SMTP_*` / `MAIDAN_SLACK_*` / `MAIDAN_GITHUB_*` and create the apps |
| "Client SDKs" | Four 0.1.0 clients (TypeScript, Python, Go, Rust) to the frozen v1 contract, each black-box-verified (`scripts/sdk-test.sh`) + a report-only `sdk interop` CI job | Shipped (0.1.0, early) |

## What "audited" covers

Every successful authenticated change leaves an attributed record — who acted,
and on whose behalf — and the privileged ones leave a named audit row.

- **Privileged actions have named audit rows** — 40 action kinds: token and
  app-token mint, delegation and revoke; delegation grants and policy; share
  tickets; channel membership; member freeze; SCIM provisioning; secrets and
  egress targets; legal hold; message purge, workspace purge, erase, export and
  import; artifact erase; gate and review-requirement clears; delivery, outbox
  and automation replays; reindex. `audit_coverage_e2e` and `authority_audit_contract` exercise them.
- **Authority changes fail closed.** Tokens, grants, share tickets, the grant
  ceiling, purge, erase, import and legal hold write their audit row inside the
  change's own transaction, so a failed audit write aborts the change
  (`authority_audit_contract`). Routine rows are best-effort: a failed write is
  counted in `maidan_audit_write_failures_total` and pages
  `MaidanAuditWriteFailures` on the first.
- **Every other mutation is attributed.** A successful `POST`/`PUT`/`PATCH`/`DELETE`
  that wrote no attributed event or audit row of its own gets a generic
  `mutation` row (operation, path, status). Ordinary content — posts, edits,
  reactions — is recorded in the event log, which is durable, ordered and
  replayable. MCP records per tool call.
- **Denials are counted, not stored**, with one exception. Anonymous and ordinary
  401/403s go to `maidan_authorization_decisions_total` and sampled logs, since an
  attacker-controlled request stream would otherwise be an unbounded write
  amplifier against the audit table. A denial of a **delegated** actor — a named
  member holding an expiring, revocable grant — is stored as
  `authorization.decision`.
- **Reads are not audited**, except the reads that release a whole workspace or a
  secret (`workspace.export`, `secret.resolve`, `legal_hold.preserved_read`),
  which write their row before releasing anything. `audit:read-global` and
  `operator:global` bound who *can* read across tenants.

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
  scale-out smoke, promtool, otlp smoke) gate every merge; the non-required `a2a tck` job and the report-only
  `sdk interop` job prove the client/interop surface without blocking.
- Release artifacts are cosign-signed; verify before trusting a tag
  ([SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release)).
