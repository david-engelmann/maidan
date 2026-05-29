# Remaining work (post–Product Ladder 17–34)

Exhaustive backlog after Clusters **17–27** and **28–34** (`v17.0.0` → `v34.0.0`). Use with
[[Open Work]] (standing risks + short deferrals), [[Product Completion Checklist]], and the
forward plan [[Clusters/Product Ladder 35+]].

**Latest ladder close:** [[Retros/Product Ladder 30-34]] — tags **`v30.0.0`–`v34.0.0`** (PRs #202–#206).

---

## 1. Incomplete or partial implementations (honest gaps)

These shipped in 23–27 but are **not** full parity with the cluster kickoff text,
upstream specs, or Slack-grade product depth.

| Area | Shipped | Gap |
|------|---------|-----|
| **MCP streamable HTTP (27→35)** | Bidirectional mux at **`v35.0.0`**: open SSE session + follow-up `POST` on `Mcp-Session-Id` | Not full MCP 2024-11-05 transport (no session TTL, no `GET` upgrade path). |
| **Web UI (23)** | Vanilla `/ui` tabs: events, search, thread FSM, token mint | No channel list, no create-channel/thread flows, no WS live tail, no artifact upload UI, no federation admin, no purge UI, no faceted search controls, no React/Vite app (deferred since Cluster H). |
| **Helm (24→32)** | `helm/maidan` + **`helm/maidan-stack`** optional Postgres/MinIO at **`v32.0.0`** | No ingress/cert-manager bundle; no `helm install` CI (template smoke only). |
| **Workspace purge (25→31)** | Deep purge through **`v31.0.0`**: messages, embeddings, references, tokens, events, artifact metadata + blobs | Does **not** delete members, channels, threads, workspace row, peers, or OIDC identities. |
| **Product gate (26)** | Checklist + lightweight e2e | Does not prove compose multi-instance federation, MinIO multipart at scale, or Postgres+Helm e2e deploy. |
| **Capabilities (22)** | Denial matrix for five paths | Not exhaustive positive/negative coverage of every route and MCP tool. |
| **A2A (21)** | `SendMessage`, `GetTask` | No `SendStreamingMessage`, push configs, or agent card discovery beyond well-known hints. |
| **Router (20)** | `resolve_*` for HTTP/MCP | No mention routing policies, channel default subscriptions, or push notifications. |
| **MCP resource fan-out (17→33)** | Tools + HTTP tombstone + FSM at **`v33.0.0`** | Other HTTP mutations (edit, purge) still omit notifications. |
| **Delivery cursors (13)** | Postgres `maidan_delivery_cursor` | SQLite impl is **no-op** (`Ok(0)` / passthrough). |
| **Outbox (10–12)** | Postgres transactional outbox + quarantine | SQLite outbox **shipped** (14) but poison-row **manual** recovery API still absent; NOTIFY remains at-most-once. |
| **mcp-stdio** | SQLite-only CLI transport | **Postgres `mcp-stdio` deferred** since Cluster H. |
| **Semantic search (18)** | SQLite brute-force cosine | No `sqlite-vec` / HNSW; large workspaces will not scale on SQLite. |
| **Embeddings** | Pluggable provider | Default **`hash-v1`** is not semantic; per-model **table split** deferred (mixed dimensions). |
| **Search scores** | Lexical + semantic per backend | **Score normalization** across Postgres vs SQLite ranks not unified. |
| **Message edit** | `PATCH /messages/:id`, MCP `edit_message`, `MessageEdited` bus event (v29) | Edit history / UI affordance still deferred. |
| **Pinned content** | Architecture mentions pins | **No pin API** or UI. |
| **Bootstrap hardening** | `MAIDAN_BOOTSTRAP=1` gate | Threat model: **compile-time removal** of bootstrap routes **not implemented**. |

---

## 2. Standing risks (still open)

From [[Open Work]] — unchanged by 23–27 except where noted.

- **At-most-once event bus** — outbox + relay reduce loss; NOTIFY duplicates / gaps still possible; clients must idempotent-replay.
- **Bootstrap / `AUTH_DISABLED` exposure** — misconfiguration in prod is catastrophic.
- **Indexer staleness** — opt-in via `INDEXER_STALE_SECS`.
- **PostgresBus listener recovery** — best-effort; `/health/ready` reflects retry state.
- **Low coverage floor** — CI **11%** line minimum; depth is opportunistic.
- **`hash-v1` deployments** — semantic search quality depends on configuring a real embedding provider.

---

## 3. Deferred from retros (no owner / post-ladder)

| Item | Source | Notes |
|------|--------|-------|
| Per-model embedding tables | Cluster 5.0 / Open Work | Filter by model at query time today. |
| `sqlite-vec` SQL functions | Cluster 18.0 retro | sqlx/extension linkage blocked brute-force path. |
| Schema parity property test (`information_schema`) | Cluster A retro | Cross-dialect DDL drift risk. |
| SQLite file-backed durability tests | Track V retro | |
| Sigstore/cosign release artifacts | Track V.3 / Operations | Manual today. |
| Client-side 5 MiB S3 multipart chunking runbook | Cluster 19.0 retro | Operator docs thin. |
| `SendStreamingMessage` (A2A) | Cluster 21.0 retro | |
| GitHub Pages enablement | Open Work | mdBook builds; site may be off in repo settings. |
| Post-quarantine outbox replay API | Cluster 12.0 retro | Manual DB intervention only. |
| OTLP dashboards / SLO wiring | Track T / Product checklist | Observability crate exists; dashboards not productized. |
| Multi-region active-active | Ladder 17–27 out of scope | |

**Closed by 17–27 (remove from active deferrals):** S3 multipart, MCP HTTP notifications (16), full ladder Helm chart (24), workspace message purge API (25), MCP streamable subset (27), router + A2A RPC path (20–21).

---

## 4. Slack parity matrix (aspirational)

Maidan is **Slack-shaped**, not Slack-complete. Use this when prioritizing product work.

| Slack capability | Maidan today | Gap / notes |
|----------------|--------------|-------------|
| Workspaces / orgs | Single workspace per token; OIDC binds user→workspace | No org hierarchy above workspace ([[OIDC]] non-goal). |
| Channels (public/private) | `Channel` + `private` flag | No channel archiving UI, shared channels, or Slack Connect-style cross-org channels (federation is event replication, not shared channel UX). |
| Direct messages | — | **No DMs** — agents/humans only via channels/threads. |
| Threads | First-class `Thread` + FSM | No thread sidebar UI, no “also send to channel” split. |
| @mentions | `Mention` records | No highlight rules, notification prefs, or email digests. |
| Reactions | **Votes** (approval / request-changes) | Not emoji reactions; no custom emoji. |
| Message editing | PATCH + MCP | No edit history or UI. |
| Message deletion | Tombstone + purge | No “delete for me” vs “for everyone” UX; purge is admin API. |
| Pins | — | **Not implemented**. |
| Files / snippets | Artifacts (localfs / S3 multipart) | No gallery, previews, or GDrive-style integrations in UI. |
| Search | Lexical + semantic, facets on Postgres | UI: query string only; no filters in `/ui`. |
| Workflow / shortcuts | MCP tools + FSM | No slash commands, Workflow Builder, or no-code automations. |
| Apps / bots | Members with `kind` + API tokens | No OAuth app install model separate from member tokens. |
| Huddles / calls | — | **Not planned** on ladder. |
| Canvas / lists | References between entities | Not free-form docs or structured lists like Slack lists. |
| Presence / status | — | **No online/away/typing**. |
| Mobile / desktop clients | HTTP + MCP + `/ui` | No native clients; stdio MCP for desktop agents only. |
| Enterprise SSO | OIDC (Cluster 2.0) | No SAML-in-Maidan; no SCIM provisioning. |
| Compliance export | Workspace message purge + per-message purge | Incomplete workspace erasure (see §1). |
| Real-time | WS `/ws/subscribe`, MCP SSE | UI does not subscribe over WS; bus not guaranteed exactly-once. |
| Admin / audit | `GET /workspaces/:id/audit` at **`v28.0.0`** | No admin console beyond `/ui`; global audit admin API still absent. |
| Rate limits / abuse | Optional HTTP limit (`MAIDAN_RATE_LIMIT_MAX`, v30) | Per-capability quotas, MCP, distributed limiter still open. |

---

## 5. Suggested improvements (engineering)

Ordered roughly by leverage; not committed roadmap.

### Transport & agents

- Full MCP streamable HTTP session (bidirectional mux, align with 2024-11-05 transport).
- `mcp-stdio` against Postgres for prod-like local agents.
- A2A streaming + task push; federate A2A with external agent runtimes.
- WS schema versioning and typed client SDKs (called out Cluster 4.0 retro).

### Data & search

- Workspace **full erasure** job: embeddings, artifacts, events, members, peers.
- `sqlite-vec` or migrate dev to Postgres for semantic scale tests.
- Per-model embedding tables; reindex tooling when provider changes.
- Unified search score reporting across backends.

### Ops & deploy

- Helm **umbrella** chart or subcharts for Postgres + MinIO + server.
- `helm install` smoke in CI (kind/k3d).
- Automate cosign on release workflow.
- Outbox quarantine replay / admin API.
- Raise coverage floor incrementally (target 20%+).

### UI & product

- Real-time event tail in `/ui` via WS or SSE.
- Channel browser, create flows, artifact upload, purge confirm.
- Faceted search controls; thread-focused layout.
- Optional React/Vite SPA (Cluster H deferral).

### Security & compliance

- HTTP rate limits and capability-scoped quotas.
- Compile-time bootstrap strip for release builds.
- List/filter audit events API for operators.
- SCIM / SAML via IdP only (document patterns).

---

## 6. Documentation & process debt

| Item | Status |
|------|--------|
| Retro + tag per cluster **23–27** | Not done until maintainer runs retro PRs and tags |
| `docs/Capabilities.md` sections v23–v27 | Pending retro PR |
| `docs/Roadmap.md` “current cluster” | Still points at 23.0 — update to post-ladder |
| `docs/Architecture.md` “What's deliberately not here” | Stale (still says GDPR Cluster V only) |
| `docs/Open Work.md` deferrals table | Still lists Helm / streamable HTTP as open — **refresh** |
| Cluster retros **23.0–27.0** | Missing under `docs/Retros/` |
| [[Clusters/Product Ladder 17-26]] | Superseded by 17–27; keep for history or archive |

---

## 7. Codebase TODOs / stubs (scan)

No widespread `todo!()` / `unimplemented!()` in library crates. Notable **intentional stubs**:

- `maidan-store` SQLite: `get_delivery_cursor` / `advance_delivery_cursor` no-ops.
- `maidan-cli mcp-stdio`: bails on non-SQLite `DATABASE_URL`.
- `Threat-Model.md`: bootstrap routes compile-time disable **not implemented**.

---

## 8. How to use this file

1. **Pick a theme** (Slack parity row, §1 partial, or §3 deferral).
2. **Open a cluster or track issue** per [[Operations]].
3. On ship: move items to [[Capabilities]], [[CHANGELOG]], and trim §3 here via retro PR.

See also: [[Open Work]], [[Roadmap]], [[Post-1.0]], [[Clusters/Product Ladder 17-27]],
[[Clusters/Product Ladder 35+]].
