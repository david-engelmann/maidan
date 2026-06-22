# Remaining work (post–`maidan-agent-1.0`)

Exhaustive backlog after Product Ladders **17–34**, **35–58** (`maidan-2.0`), **59–67**, **68–76** (agent substrate), **77–101** (operator), and **102–120** (scale-out & hardening, gate `maidan-scale-1.0`).
Use with [[Open Work]] (standing risks + short deferrals), [[Product Completion Checklist]],
[[Clusters/Product Ladder 77+]] (Clusters **77–101**), and [[Clusters/Product Ladder 102+]] (Clusters **102–120**).

**Latest closes:** **`v120.0.0`** scale product gate (`maidan-scale-1.0`); Product Ladder 102+ (Clusters 102–120) **complete**.

---

## 1. Incomplete or partial implementations (honest gaps)

| Area | Shipped | Gap |
|------|---------|-----|
| **MCP streamable (73)** | `POST/DELETE /mcp/streamable`, TTL, mux (**78**) | — |
| **Web UI (23)** | `/ui` tabs: events, search, FSM, tokens | Channel browser, WS tail, artifacts (**92–96**); no React SPA in ladder **77+** |
| **Helm (55)** | `helm/maidan` + `helm/maidan-stack`, kind CI | Production value profiles (**88**) — in progress |
| **Workspace erasure (53)** | `DELETE /workspaces/:id` full erase | Does not cover org-level IdP user deletion (use IdP) |
| **Capabilities (69)** | Full MCP matrix + sample HTTP contract | Every OpenAPI path in CI (**77**) |
| **A2A (72)** | Persisted tasks + cancel/progress (**79**) | Task marketplace UI |
| **Context (74)** | HTTP + MCP context + cursors (**82**) | Store-level workspace thread cursor |
| **Delivery cursors (13)** | Postgres + SQLite cursors (**56**, **83**) | — |
| **Outbox** | Relay modes + quarantine + HTTP replay (**56**, **84**) | — |
| **mcp-stdio (36)** | Postgres-backed | Embedded indexer mode (**100**) |
| **Semantic (75)** | CLI reindex + runbook; Postgres HNSW; optional `sqlite-vec` (**85**) | — |
| **Embeddings** | Pluggable provider; per-model tables + `embedding_model` query (**86**); operator reindex jobs (**87**) | Durable job store |
| **Bootstrap** | `MAIDAN_BOOTSTRAP=1` gate | Compile-time strip (**91**) |
| **Observability (76)** | Agent metrics runbook + gate e2e | OTLP export + dashboards (**89–90**) |
| **Delivery ops (68)** | Unified operator deliveries API (**80**) | — |

**Closed since older drafts of this file:** pins API (**40**), slash commands (**51**), FSM hooks (**52**), DMs (**39**), message edit (**29**), outbox HTTP replay (**56**), Helm stack (**32**/**55**), workspace full erase (**53**), A2A `SendStreamingMessage` (**37**), MCP resource fan-out (**38**), capability matrix for all MCP tools (**69**).

---

## 2. Standing risks (still open)

From [[Open Work]] — unchanged except where a release mitigated.

- **At-most-once event bus** — outbox + relay; clients must idempotent-replay by `log_id`.
- **Bootstrap / `AUTH_DISABLED` misconfiguration** — catastrophic in production.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener recovery** — best-effort; `/health/ready` reflects retry state.
- **Coverage floor ≥40%** — enforced in CI over the full suite (v114); opportunistic depth increases beyond the floor.
- **`hash-v1` default** — `openai-compatible` provider (v117) for real semantics; `hash-v1` is the offline/dev default.
- **Tag backlog** — `v93.0.0`–`v100.0.0` + the `maidan-operator-1.0` gate tag remain uncut.

---

## 3. Deferred from retros (no owner / post–69)

| Item | Notes |
|------|-------|
| Per-model embedding tables | Filter by model at query time today (**75**) |
| `sqlite-vec` / HNSW on SQLite | Extension linkage deferred |
| Schema parity property test | Cluster A retro |
| Sigstore/cosign release artifacts | Manual (**Operations**) |
| OTLP dashboards / SLO wiring | **89–90** ([[Clusters/Product Ladder 77+]]) |
| Multi-region active-active | Out of scope |
| OpenAPI-wide capability map | Cluster **69** shipped MCP + samples only |
| Unified webhook + automation delivery tables | **68** kept separate queues |

---

## 4. Slack parity matrix (aspirational)

Maidan is **Slack-shaped**, not Slack-complete.

| Slack capability | Maidan today | Gap / notes |
|------------------|--------------|-------------|
| Workspaces / orgs | Workspace-scoped tokens + OIDC | No org hierarchy above workspace |
| Channels | Public/private `Channel` | No Slack Connect-style shared-channel UX |
| Direct messages | Thread-backed 1:1 DMs (**39**) | No group DMs |
| Threads | `Thread` + FSM | No “also send to channel” UX |
| @mentions | `Mention` records | No notification router / email digests |
| Reactions | Votes + emoji reactions API | Not full emoji picker UX |
| Message editing | PATCH + MCP + edit history in context (**67**) | Limited `/ui` affordance |
| Pins | Pin/unpin HTTP + MCP | No dedicated pins UI |
| Files | Artifacts + multipart S3 | No gallery / Drive integrations in UI |
| Search | Lexical + semantic + facets | `/ui` query string only |
| Workflow / shortcuts | Slash commands + FSM hooks + webhooks | No Workflow Builder UI |
| Apps / bots | Installed apps + OAuth (**57**, **65**) | Distinct from member tokens |
| Huddles / calls | — | Not planned |
| Presence / status | — | Not implemented |
| Mobile / desktop | HTTP + MCP + `/ui` | No native clients |
| Enterprise SSO | OIDC | SAML-in-Maidan / SCIM not in scope |
| Real-time | WS + MCP SSE | UI does not WS-tail; bus not exactly-once |
| Admin / audit | Workspace audit + automation DLQ (**68**) | No global admin console |

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

- `maidan-store` SQLite: delivery cursor no-ops.
- Threat model: compile-time bootstrap disable not implemented.

---

## 8. How to use this file

1. Pick a row from §1, §3, or [[Clusters/Product Ladder 77+]].
2. Open a cluster issue per [[Operations]].
3. On ship: update [[Capabilities]], [[CHANGELOG]], trim via retro PR.

See also: [[Open Work]], [[Roadmap]], [[Agent Integration]].
