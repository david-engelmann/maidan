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
| `maidan-operator-1.0` | `v101.0.0` | Operator surface: the web UI, health, metrics, OpenAPI (`maidan_operator_gate_e2e`) |
| `maidan-scale-1.0` | `v120.0.0` | Scale-out (multi-replica, sharded fan-out, SLOs) |

All four gate tags are cut. Post-120 work ships on the same `vX.0.0` ladder as
post-gate hardening (no new gate tag).

## Claims → evidence

| Claim (README / site) | Evidence | Status |
|-----------------------|----------|--------|
| "Durable, shared memory: threads, results, artifacts, tool-call transcripts, all searchable" | `maidan-store` (Postgres + SQLite `Store` parity, `backend_parity` test); content-addressed artifacts; `thread_results`; `tool_transcript`; full-text (`tsvector`/FTS5) + semantic (`pgvector`) search | Shipped |
| "Tasks with dependencies, skill-based claiming, assignment + leases, scheduled runs, blocking waits" | Task-DAG + queue, scheduled/recurring tasks, skill routing, coordination waits (`wait_for_ready`/`wait_for_result`) — store tests `thread_deps`, `skill_routing`, `task_schedules`, `run_ready_dependents_suite`; e2es `thread_dependencies_e2e`, `thread_result_e2e` | Shipped |
| "Pull exactly the context a step needs — far fewer tokens" | Thread/workspace context packs (lean edits by default, `include_edits` opt-in); `snippet_only` search; capability-filtered `tools/list`; opt-in lean event frames; omit-empty metadata. **Measured: a scoped pack is ~6.8× fewer tokens than dumping the whole channel** (`token_pack` harness → [Benchmark.md](Benchmark.md#context-pack-token-savings-token_pack)) | Shipped + measured |
| "Access is scoped on every token; private channels enforced on reads, events, and search; **privileged** actions are audited" | Capability model (every route + tool checks caps); per-channel/thread RBAC — e2es `channel_access_e2e`, `dm_participation_e2e`; filtered-ANN search excludes private channels in-query; subscribe-grant enforcement; every successful change made with a Maidan token or session attributed, unless its only record is a best-effort generic audit row and that write fails; 52 named audit actions, authority changes written transactionally (fail-closed) — `audit_coverage_e2e` | Shipped — see the audit-scope note below, including the changes it does not cover |
| "Speaks MCP, REST, and WebSocket over one data model and one login" | One `AppState`/`Store`; REST (OpenAPI 3.1, `openapi_e2e` bijection), MCP (JSON-RPC + streamable HTTP), WebSocket subscribe — all bearer-authed | Shipped |
| "MCP-native — an MCP client connects directly and gets typed tools + live notifications" | `POST /mcp` + streamable HTTP; MCP `2026-07-28` (negotiated, default) with `2024-11-05` fallback; `resources/updated`; live-verified LangChain + AutoGen recipes (`docs/Framework Integrations.md`) | Shipped |
| "A pasted token is exchanged for an HttpOnly session with that token's authority; an unsafe session request or socket from another origin is refused" | `crates/maidan-server/tests/session_from_token_e2e.rs`: `a_pasted_token_becomes_a_session_with_that_tokens_authority`, `a_cross_origin_unsafe_request_on_a_session_is_refused`, `a_cross_origin_socket_cannot_use_a_session`, `only_a_bearer_is_exchanged_and_sign_out_ends_the_session`. A request with neither `Origin` nor `Sec-Fetch-Site` is accepted and is not claimed to be origin-safe. MCP stays bearer-only in `token_session`; that exclusion is not asserted by the e2e | Shipped; MCP exclusion not yet covered by `session_from_token_e2e` |
| "Single static binary, laptop SQLite → multi-replica Postgres cluster" | One binary selected by `DATABASE_URL`; `scale-out smoke` required CI job; workspace-sharded fan-out; LSN causal read-replica routing (`read_routing` e2e vs real streaming replication) | Shipped (`maidan-scale-1.0`) |
| "Built to be run, not just demoed — probes, Prometheus, OTLP, durable event log + replay, cross-replica correctness" | `/health/{live,ready}`; `/metrics`; `otlp smoke` + `promtool (alert rules)` required CI; transactional outbox (events commit atomically with their domain write); **leased outbox claim so N replicas relay each row once** (`concurrent_relays_claim_disjoint_outbox_rows`); self-healing NOTIFY floor (`notify_floor::sim` unit test, 400 seeds by default, replayed with `MAIDAN_SIM_SEED`) | Shipped |
| "Signed release artifacts" | Keyless cosign bundles on every release and cosign signatures on every image digest (`release.yml`); per-arch tarballs SHA-256-pinned in the quickstart image. A CycloneDX SBOM per image, attested to its digest and published beside the tarballs, starts with the first tag after v412.0.0: no earlier release has one, because the old SBOM step never produced a file. Verify: see [SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release) | Signatures shipped; SBOMs from the next tag |
| "A2A transport" | A2A v1.0 over **JSON-RPC, REST §11 and gRPC §10**, all complete. The gRPC binding (opt-in) serves the official `lf.a2a.v1.A2AService` from the unmodified v1.0.1 `a2a.proto`, every operation over the same handlers as the other two. Agent Card §4.4.1. The official A2A TCK runs over all three bindings in the non-required `a2a tck` CI job; exclusions are listed in `scripts/a2a-tck/exclusions.txt` | Shipped (all three bindings) |
| "Off-platform reach: notifications, email, Slack, GitHub" | Per-recipient notification ledger + router + unified inbox; SMTP transport + durable mail retry queue (outbox + worker + DLQ); Slack + GitHub projectors (bidirectional, loop-safe) | **Shipped, config-gated** — inert until you set `MAIDAN_SMTP_*` / `MAIDAN_SLACK_*` / `MAIDAN_GITHUB_*` and create the apps |
| "Client SDKs" | Four 0.1.0 clients (TypeScript, Python, Go, Rust) to the frozen v1 contract, each black-box-verified (`scripts/sdk-test.sh`) + a report-only `sdk interop` CI job. Typed responses and an error per problem type are on `main` and pass the same script, but are not yet published | Shipped (0.1.0, early); typed surface unreleased |
| The published server image has no HTTP bootstrap routes | `crates/maidan-server/Dockerfile` defaults `MAIDAN_ENABLE_BOOTSTRAP` to `0` and then builds `--no-default-features`. `.github/workflows/release.yml` (`build + push maidan-server`) passes only `MAIDAN_VERSION`, so the published image keeps that default. CI job `bootstrap compile-time strip` fails a default build that still compiles the routes in | Shipped |
| A production Helm render refuses a development image, and placeholders only where the chart looks | `scripts/helm-template-smoke.sh`. A production render refuses `image.repository: maidan-server`, a `dev`/`latest`/empty tag without `image.digest`, and, unless `existingSecret` is set, an unset or development `DATABASE_URL` or an empty `secrets` value. `config`, `image.tag` and `image.digest` holding `CHANGE_ME` fail every render; `secrets` and `contentKek` holding it fail only when `existingSecret` is unset. `existingSecret` skips those checks and does not prove the Secret exists or holds its keys | Shipped |

## Behavior the reference docs state

These are not README slogans. They are sentences in the reference docs, and
each maps the same way: a test, the code that does it, or an honest gap.

| Claim | Evidence | Status |
|-------|----------|--------|
| With OIDC off, `MAIDAN_SUBSCRIBE_RESUME_SECRET` lets the server start without `MAIDAN_SESSION_SECRET` | `main.rs` calls `subscribe_resume::secret_from_env()` only when OIDC is off. That function uses `MAIDAN_SUBSCRIBE_RESUME_SECRET` when set, otherwise the session secret. With authentication off and neither secret set, startup uses the built-in test secret. OIDC still refuses to start without `MAIDAN_SESSION_SECRET` (`oidc::OidcSettings::from_env`) | Shipped — no test covers the env choice |
| `Maidan-Room-LSN` is the caller's workspace head; a narrower tap waits on its own shape's head | `the_header_reports_the_callers_room_not_the_instance` (`room_lsn_scope_e2e`) stamps the caller's room, not the instance. A tap compares history to the head it was given (`history_caught_up`, `live_waits_for_workspace_head_not_global_room`). For a channel, thread or kind filter that head is the shape's own high-water, not the workspace header. The server does not publish a second header for the shape | Shipped |
| A lapsed lease returns within a reaper tick (5 s by default), and one replica frees at most 1,000 claims per tick | Tick default: `the_reaper_is_on_by_default_and_zero_turns_it_off`. The store sweep stops at the limit it is given and leaves the rest (`a_batch_is_bounded_and_takes_the_oldest_deadline_first` in `reap_expired_claims_sqlite` / `_postgres`). The replica cap is `MAX_PER_TICK` (1,000) in `claim_reaper.rs` | Shipped — no test fills the 1,000 cap |
| `assign_thread` and `claim_thread` carry no lease, and `assign_thread` ends the previous hold | `a_new_holder_never_inherits_a_lease_deadline_sqlite` / `_postgres`: releasing and then calling `claim_thread` or `assign_thread`, or assigning over a live lease, leaves the new holder no deadline, and it keeps the thread past the old one. `reap_expired_claims_*` leaves a claim with no lease alone | Shipped |
| Review and land-gate history starts at migration 0121; verdicts overwritten before it were not kept | `decision_history_keeps_every_verdict_sqlite` / `_postgres` drops the history tables, re-runs 0121, and checks the backfill of the rows that existed then | Shipped |
| A change request that does not send work back still notifies the last worker | `every_review_verdict_appends_review_submitted_with_the_last_worker_sqlite` / `_postgres` records the worker on a change request that sends nothing back. The notification router notifies `ReviewSubmitted` / `RequestChanges` whenever that worker is set; it does not look at whether the thread was sent back | Shipped — no test asserts the inbox row for the non-send-back case |
| A tombstone leaves ciphertext in the database, its exports and every peer that ingests it. A pre-shred backup read with `MAIDAN_CONTENT_KEK`, and a peer that never ingests the tombstone, still have the words | `withdrawn_words_are_unreadable_on_every_surface` and `peers_get_ciphertext_only_for_shredded_words` (`crypto_shredding_e2e`). The two surviving copies are Threat Model T7; neither is exercised by a test | Shipped for the shred; the two copies are the threat model, not a test |

## What "audited" covers

Every successful change made with a Maidan token or browser session leaves an
attributed record — who acted, and on whose behalf — and the privileged ones
leave a named audit row, unless the change wrote no event or audit row of its
own and the best-effort generic row fails to write. Authority changes are not
on that path: a failed audit write aborts the change. Two kinds of change are
outside attribution entirely, listed under "Every other mutation" below.

- **Privileged actions have named audit rows** — 52 action kinds besides the
  generic `mutation` row (counted from the `action` names the server writes):
  token and app-token mint, rotation, delegation and revoke; browser sign-in and
  sign-out; delegation grants and policy; share tickets; channel membership;
  member freeze; SCIM provisioning; skill governance; secrets and egress
  targets; legal hold; retention policy; message purge, workspace purge, erase,
  export and import; artifact erase; app revoke; reviewer removal and
  review-requirement changes; gate and review-requirement clears; delivery,
  outbox and automation replays and result-delivery attempts; reindex; and
  denials of a delegated actor (below). `audit_coverage_e2e` and
  `authority_audit_contract` exercise them.
- **Authority changes fail closed.** Tokens, grants, share tickets, browser
  sessions, the grant ceiling, purge, erase, import and legal hold write their audit row inside the
  change's own transaction, so a failed audit write aborts the change
  (`authority_audit_contract`). Routine rows are best-effort: a failed write is
  counted in `maidan_audit_write_failures_total` and pages
  `MaidanAuditWriteFailures` on the first.
- **Every other mutation is attributed, best-effort.** A successful `POST`/`PUT`/`PATCH`/`DELETE`
  that wrote no attributed event or audit row of its own gets a generic
  `mutation` row (operation, path, status). That write is the best-effort path
  above: if it fails, the change still stands and has no attributed record. The
  row is skipped when
  `contracts/http-operation-kinds.json` classifies it as a read
  (`http_operation_kinds_e2e` checks that each one writes nothing). Ordinary
  content — posts, edits, reactions — is recorded in the event log, which is
  durable, ordered and replayable. MCP records per tool call and A2A per method
  on every binding, gRPC included, so a read or a refused call is not recorded
  as a change (`a2a_operation_kinds_e2e`). Two kinds of change are not covered.
  Slack and GitHub ingress (`/integrations/slack/events`,
  `/integrations/github/events`) is authenticated by the sender's request
  signature, not a Maidan credential: the message posts as the member the
  channel link names, and its event carries no attribution (no actor or
  subject). And `GET /mcp/stream` and `GET /ws/subscribe` advance a named
  consumer's delivery cursor (the WebSocket also stamps a member's last-seen
  time) with no event or audit row; `contracts/http-operation-kinds.json` gives
  the reasons.
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
- **SDKs are 0.1.0.** Usable against the frozen v1 contract, dependency-light, but early.
  Typed responses and an error per problem type are on `main`
  (`scripts/sdk-test.sh`) but not yet tagged, so the published packages return
  plain JSON and raise one error type; registry-published interop CI is a
  follow-up.
- **Not an orchestration planner or an agent runtime.** Maidan does not run your models
  or decide how an agent reasons. It is the durable place agents coordinate.

## How this stays honest

- New public claims add a row here in the same PR.
- The required CI checks (lint, secrets scan, unit, integration, docker-compose smoke,
  scale-out smoke, promtool, otlp smoke) gate every merge; the non-required `a2a tck` job and the report-only
  `sdk interop` job prove the client/interop surface without blocking.
- Release artifacts are cosign-signed; verify before trusting a tag
  ([SECURITY.md](https://github.com/david-engelmann/maidan/blob/main/SECURITY.md#verifying-a-release)).
