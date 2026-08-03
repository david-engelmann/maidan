# Remaining work (post–`maidan-agent-1.0`)

Exhaustive backlog after Product Ladders **17–34**, **35–58** (`maidan-2.0`), **59–67**, **68–76** (agent substrate), **77–101** (operator), and **102–120** (scale-out & hardening, gate `maidan-scale-1.0`).
Use with [[Open Work]] (standing risks + short deferrals), [[Product Completion Checklist]],
[[Clusters/Product Ladder 77+]] (Clusters **77–101**), and [[Clusters/Product Ladder 102+]] (Clusters **102–120**).

**Latest closes:** **`v120.0.0`** scale product gate (`maidan-scale-1.0`); Product Ladder 102+ (Clusters 102–120) **complete**. Post-gate hardening **121–126** (`v121`–`v126`): OpenAPI capability map in CI, promtool-executed SLO rules, OTLP e2e, 8 required checks, and opt-in at-least-once delivery (WebSocket + MCP SSE). This file was **reconciled against code at v126** (Cluster 127) — ~11 entries that the docs listed as open were already shipped — and again at **v143** (Cluster 144): the §4 admin-audit gap shipped in **132**, and the `/ui` track (**134–143**) surfaced the collaboration + operator features.

---

## 1. Incomplete or partial implementations (honest gaps)

| Area | Shipped | Gap |
|------|---------|-----|
| **MCP streamable (73)** | `POST/DELETE /mcp/streamable`, TTL, mux (**78**) | — |
| **Web UI (23)** | `/ui` tabs: events feed + WS tail, search, thread/FSM, tokens, admin/audit (**92–96**); reactions/pins/message-timestamps + inline slash results (**134/135/143**), DMs + group DMs (**136/139**), presence roster (**140**), Operator console — deliveries/DLQ + global audit + reindex (**137/138**), Slash registry (**142**) | No React SPA (non-goal in ladder **77+**) |
| **Helm (55)** | `helm/maidan` + `helm/maidan-stack`, kind CI; production value profiles `values-profile-{otel,redis,s3}.yaml` + `PROFILES.md` (**88**) | — |
| **Workspace erasure (53)** | `DELETE /workspaces/:id` full erase | Does not cover org-level IdP user deletion (use IdP) |
| **Capabilities (69)** | Full MCP matrix + bidirectional HTTP capability-map contract; every OpenAPI op classified bearer/session/public in CI (**121**) | — |
| **A2A (72)** | Persisted tasks + cancel/progress + `tasks/resubscribe` (**79**) | Task marketplace = product/UI (§4), not a backend gap |
| **Context (74)** | HTTP + MCP context + cursors (**82**); store-level `page_threads_for_workspace` keyset cursor | — |
| **Delivery cursors (13)** | Postgres + SQLite cursors (**56**, **83**) | — |
| **Outbox** | Relay modes + quarantine + HTTP replay (**56**, **84**) | — |
| **mcp-stdio (36)** | Postgres-backed line-delimited JSON-RPC | No dedicated embedded-indexer mode (indexer runs as a server background task; niche, low value) |
| **Semantic (75)** | CLI reindex + runbook; Postgres HNSW; optional `sqlite-vec` (**85**) | — |
| **Embeddings** | Pluggable provider; per-model tables + `embedding_model` query (**86**); operator reindex jobs (**87**); durable job store — `maidan_reindex_jobs`, status resolvable on any replica (**104**) | — |
| **Bootstrap** | `MAIDAN_BOOTSTRAP=1` gate + compile-time strip — `bootstrap` cargo feature; `--no-default-features` removes the routes, asserted by `bootstrap_absent_e2e` + the `bootstrap-strip` CI job (**91**) | — |
| **Observability (76)** | Agent metrics runbook + gate e2e; OTLP export — traces + metrics fanout (**89**, env-gated, documented in `Production.md`), asserted end-to-end against a real collector (**123**); SLO recording/alert rules + operator dashboard (**90**), extended to scale-out indexer metrics (**121**) and promtool-executed in CI (**122**) | — |
| **Delivery ops (68)** | Unified operator deliveries API (**80**) | — |

**Closed since older drafts of this file:** pins API (**40**), slash commands (**51**), FSM hooks (**52**), DMs (**39**), message edit (**29**), outbox HTTP replay (**56**), Helm stack (**32**/**55**), workspace full erase (**53**), A2A `SendStreamingMessage` (**37**), MCP resource fan-out (**38**), capability matrix for all MCP tools (**69**).

---

## 2. Standing risks (still open)

From [[Open Work]] — unchanged except where a release mitigated.

- **At-most-once event bus (default path)** — the optimistic live path (`forward_bus_items`) is best-effort; **mitigated** by opt-in `at_least_once` reconcile delivery on WebSocket (**125**) and MCP SSE (**126**), which guarantees gap-free at-least-once per `consumer_id`. Default subscribers still idempotent-replay by `log_id`.
- **Bootstrap / `AUTH_DISABLED` misconfiguration** — catastrophic in production; compile-time strip (**91**) removes the path entirely in hardened builds.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener recovery** — best-effort; `/health/ready` reflects retry state.
- **Coverage floor ≥40%** — enforced in CI over the full suite (v114); opportunistic depth increases beyond the floor.
- **`hash-v1` default** — `openai-compatible` provider (v117) for real semantics; `hash-v1` is the offline/dev default.
- **No `v93`–`v100` tags** — clusters 93–101 shipped as one batch (PR #264) → `v101.0.0`; not a backlog. All four gate tags cut.

---

## 3. Deferred from retros (no owner / post–69)

| Item | Notes |
|------|-------|
| ~~Per-model embedding tables~~ | **Closed (86/migration 0020):** per-model tables via `table_name_for_model` + `maidan_embedding_models` registry. |
| ~~`sqlite-vec` / HNSW on SQLite~~ | **Closed (85):** `sqlite-vec` optional feature + CI job; Postgres HNSW; SQLite brute-force cosine fallback. |
| ~~Schema parity property test~~ | **Closed:** `backend_parity.rs` (migration-slug lockstep) + `dialect_parity.rs` (cross-backend result parity). |
| ~~Sigstore/cosign release artifacts~~ | **Closed:** `release.yml` keyless `cosign sign-blob --bundle` over tarballs + SBOM. |
| ~~OTLP end-to-end collector smoke~~ | **Closed (123):** the `otlp` compose profile + `otlp smoke` CI job assert traces + metrics reach a real collector. |
| Multi-region active-active | Out of scope |
| ~~OpenAPI-wide capability map~~ | **Closed (121):** every OpenAPI op classified + bidirectional capability-map match in CI |
| ~~Unified webhook + automation delivery~~ | **Closed as substantially-addressed (131):** signing + backoff are shared (`automation_delivery` reuses `webhooks::{sign_payload, delivery_backoff}`) and the operator API is unified (`OperatorDelivery`). The two storage tables stay separate **by design** — distinct foreign keys (`maidan_webhook_deliveries`→subscriptions, `maidan_automation_deliveries`→slash/fsm); merging them is a risky migration for no functional gain. |

---

## 4. Slack parity matrix (aspirational)

Maidan is **Slack-shaped**, not Slack-complete.

| Slack capability | Maidan today | Gap / notes |
|------------------|--------------|-------------|
| Workspaces / orgs | Workspace-scoped tokens + OIDC | No org hierarchy above workspace |
| Channels | Public/private `Channel` | No Slack Connect-style shared-channel UX |
| Direct messages | Thread-backed 1:1 DMs (**39**) + **group DMs** (N-member, **97**); **/ui** "DMs" tab (**139**) + "Group DMs" tab (**136**) | — |
| Threads | `Thread` + FSM | No “also send to channel” UX |
| @mentions | `Mention` records | No notification router / email digests |
| Reactions | Votes + emoji reactions API; **/ui** chips + quick-add (**134**) | Not a full emoji-picker UX |
| Message editing | PATCH + MCP + edit history in context (**67**); **/ui** edit affordance | — |
| Pins | Pin/unpin HTTP + MCP; **/ui** per-message toggle (**135**) | No dedicated pins-only panel |
| Files | Artifacts + multipart S3 | No gallery / Drive integrations in UI |
| Search | Lexical + semantic + facets | `/ui` query string only |
| Workflow / shortcuts | Slash commands + FSM hooks + webhooks; **/ui** "Slash" registry tab (**142**) | No Workflow Builder UI |
| Apps / bots | Installed apps + OAuth (**57**, **65**) | Distinct from member tokens |
| Huddles / calls | — | Not planned |
| Presence / status | **Online/away + typing, cross-replica** (`PresenceHub` + `maidan_presence` NOTIFY, **103**); **/ui** "Presence" roster tab (**140**) | — |
| Mobile / desktop | HTTP + MCP + `/ui` | No native clients |
| Enterprise SSO | OIDC | SAML-in-Maidan / SCIM not in scope |
| Real-time | WS + MCP SSE; **/ui** WS-tails events + presence | Default bus not exactly-once (opt-in `at_least_once`, **125/126**) |
| Admin / audit | Workspace audit + automation DLQ (**68**); **global cross-workspace audit query API** (`GET /operator/audit`, `audit:read-global`, **132**) + **/ui** Operator tab (deliveries/DLQ **137**, audit + reindex **138**) | — |

**Classification (reconciled v143, Cluster 144):** the remaining §4 gaps are
**product/UI** features with **complete backends** — the once-"backend-tractable"
exception, a global cross-workspace **admin audit query API**, **shipped** in
Cluster **132** (`GET /operator/audit`, `audit:read-global`) and got a UI in
**138**. The **`/ui` track (134–143)** then surfaced reactions, pins, DMs, group
DMs, presence, the operator console, and the slash registry — so most §4 rows now
have a UI affordance, not just an API. What remains is deeper UX polish (full
emoji picker, pins-only panel, search facets in the UI, notification router,
Workflow Builder, file gallery), all on complete backends. **Out of scope:** org
hierarchy (workspace→org refactor), native clients, huddles, SAML/SCIM.

---

## 5. Suggested improvements (engineering)

See [[Clusters/Product Ladder 68+]] for the committed ladder (**71–76**). Opportunistic items:

- Event/subscribe contract v2 + MCP notification parity CI (**71**).
- MCP streamable complete + Agent Integration client doc (**73**).
- `maidan reindex-embeddings` or job API (**75**).
- Agent observability runbooks + **`maidan-agent-1.0`** gate e2e (**76**).

---

## 6. Documentation debt

| Item | Status |
|------|--------|
| Vault snapshot vs **`v76`** | Cluster **70** + ongoing retro discipline |
| Per-cluster retros **23–27** | Historical; capabilities in [[CHANGELOG]] |
| mdBook vs vault drift | Prefer vault + [[Agent Integration]]; mdBook follows on merge |

---

## 7. Codebase intentional stubs

_Both prior entries were stale (verified v126) and are corrected:_

- ~~`maidan-store` SQLite: delivery cursor no-ops.~~ **Implemented** — `sqlite/delivery_cursor.rs` `get_cursor`/`advance_cursor` (monotonic, `ON CONFLICT … DO UPDATE`).
- ~~Threat model: compile-time bootstrap disable not implemented.~~ **Implemented (91)** — `bootstrap` cargo feature; `--no-default-features` strips the bootstrap routes (`bootstrap_absent_e2e` + `bootstrap-strip` CI job).
- _No remaining intentional stubs in `src` — no `todo!()`/`unimplemented!()`/`FIXME` (verified v126)._

---

## 8. How to use this file

1. Pick a row from §1, §3, or [[Clusters/Product Ladder 77+]].
2. Open a cluster issue per [[Operations]].
3. On ship: update [[Capabilities]], [[CHANGELOG]], trim via retro PR.

See also: [[Open Work]], [[Roadmap]], [[Agent Integration]].
