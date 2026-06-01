# Remaining work (post–`v69.0.0`)

Exhaustive backlog after Product Ladders **17–34**, **35–58** (`maidan-2.0`), and **59–69** (agent substrate Phase XI).
Use with [[Open Work]] (standing risks + short deferrals), [[Product Completion Checklist]], and
[[Clusters/Product Ladder 68+]] (Clusters **71–76**, gate **`maidan-agent-1.0`**).

**Latest closes:** **`v69.0.0`** capability map CI · **`v68.0.0`** automation delivery · **`v67.0.0`** context packages · **`v58.0.0`** maidan-2.0 gate.

---

## 1. Incomplete or partial implementations (honest gaps)

| Area | Shipped | Gap |
|------|---------|-----|
| **MCP streamable (73)** | `POST/DELETE /mcp/streamable`, TTL (**60**) | Not full MCP 2024-11-05 bidirectional mux + documented client flow |
| **Web UI (23)** | `/ui` tabs: events, search, FSM, tokens | No channel browser, WS live tail, artifact upload UI, React SPA |
| **Helm (55)** | `helm/maidan` + `helm/maidan-stack`, cert-manager values, kind install CI | Production tuning left to operators; not a hosted SaaS |
| **Workspace erasure (53)** | `DELETE /workspaces/:id` full erase | Does not cover org-level IdP user deletion (use IdP) |
| **Capabilities (69)** | MCP tool deny + allow gate; sample HTTP contract | Not every OpenAPI path in CI |
| **A2A (72)** | RPC + streaming message + in-memory push config | Persisted push / `SubscribeToTask` |
| **Context (74)** | HTTP workspace/thread context (**67**) | MCP tools + pagination cursors |
| **Delivery cursors (13)** | Postgres `maidan_delivery_cursor` | SQLite impl is no-op |
| **Outbox** | Relay + quarantine + HTTP replay (**56**) | NOTIFY still at-most-once |
| **mcp-stdio (36)** | Postgres-backed | No bundled bus/indexer in stdio mode |
| **Semantic (75)** | SQLite brute-force + Postgres HNSW | No `sqlite-vec` HNSW; reindex job API open |
| **Embeddings** | Pluggable provider | Default **`hash-v1`**; per-model table split deferred |
| **Bootstrap** | `MAIDAN_BOOTSTRAP=1` gate | Compile-time strip not implemented |

**Closed since older drafts of this file:** pins API (**40**), slash commands (**51**), FSM hooks (**52**), DMs (**39**), message edit (**29**), outbox HTTP replay (**56**), Helm stack (**32**/**55**), workspace full erase (**53**), A2A `SendStreamingMessage` (**37**), MCP resource fan-out (**38**), capability matrix for all MCP tools (**69**).

---

## 2. Standing risks (still open)

From [[Open Work]] — unchanged except where a release mitigated.

- **At-most-once event bus** — outbox + relay; clients must idempotent-replay by `log_id`.
- **Bootstrap / `AUTH_DISABLED` misconfiguration** — catastrophic in production.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener recovery** — best-effort; `/health/ready` reflects retry state.
- **Coverage floor 11%** — opportunistic depth increases.
- **`hash-v1` default** — configure a real embedding provider for semantic quality.

---

## 3. Deferred from retros (no owner / post–69)

| Item | Notes |
|------|-------|
| Per-model embedding tables | Filter by model at query time today (**75**) |
| `sqlite-vec` / HNSW on SQLite | Extension linkage deferred |
| Schema parity property test | Cluster A retro |
| Sigstore/cosign release artifacts | Manual (**Operations**) |
| OTLP dashboards / SLO wiring | Cluster **76** |
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
| Vault snapshot vs **`v69`** | Addressed by Cluster **70** retro |
| Per-cluster retros **23–27** | Historical; capabilities in [[CHANGELOG]] |
| mdBook vs vault drift | Prefer vault + [[Agent Integration]]; mdBook follows on merge |

---

## 7. Codebase intentional stubs

- `maidan-store` SQLite: delivery cursor no-ops.
- Threat model: compile-time bootstrap disable not implemented.

---

## 8. How to use this file

1. Pick a row from §1, §3, or [[Clusters/Product Ladder 68+]].
2. Open a cluster issue per [[Operations]].
3. On ship: update [[Capabilities]], [[CHANGELOG]], trim via retro PR.

See also: [[Open Work]], [[Roadmap]], [[Agent Integration]].
