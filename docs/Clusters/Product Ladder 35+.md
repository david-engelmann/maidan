# Product Ladder 35+ — Agent-native collaboration OS

**North star:** A **world-class, agent-first** alternative to Slack — not a pixel-perfect clone,
but a **protocol-native workspace** where humans and agents collaborate with the rigor of
Linear, the reach of federated chat (Matrix/Slack Connect), the automation surface of
Zapier/GitHub Actions, and the tool depth of MCP — while keeping Maidan's load-bearing bets:
typed events, references, FSM threads, capabilities, and content-addressed artifacts.

**Versioning:** One cluster → one tag (`v35.0.0`, `v36.0.0`, …). Optional **`v2.0.0`**
product gate after Cluster **58** (see Phase VII).

**Predecessors:** [[Clusters/Product Ladder 17-27]] · [[Clusters/Product Ladder 30-34]] ·
retro [[Retros/Product Ladder 30-34]].

---

## Design principles (agent-first)

1. **Protocols over UI** — MCP, A2A, HTTP, WS are contract-first; UI consumes the same events.
2. **Every mutation is an event** — bus + audit + search indexer; agents replay from `after_id`.
3. **Capabilities, not roles soup** — fine-grained tokens; human OIDC for browser; agent tokens for tools.
4. **Structured > free text** — references, artifact kinds, votes, FSM states; LLM text is one layer.
5. **Human oversight by default** — approvals, audit, purge, rate limits; dangerous ops need `workspace:write`.
6. **Federation is replication, not shared UX (for now)** — cross-workspace event ingest; shared-channel UX is later.
7. **Ship SQLite dev / Postgres prod parity** — no stub store paths on the agent critical path.

---

## Competitive lens (what we steal)

| Platform | Pattern worth adopting | Maidan mapping |
|----------|------------------------|----------------|
| **Slack** | Channels, threads, apps, search, enterprise grid | Core shape; gaps in UI, apps, reactions |
| **Discord** | Roles, rich bots, thread auto-archive | Capability matrix + agent `MemberKind` |
| **Microsoft Teams** | Tabs, org hierarchy, meeting artifacts | Artifact kinds + future org/workspace tree |
| **Zulip** | Topic-per-thread clarity | Already thread-first; add topic labels / stream UI |
| **Matrix / Element** | Federation, room versioning | `maidan-server` federation ingest; E2E deferred |
| **Telegram** | Bot commands, inline callbacks | Slash commands + webhook callbacks (Phase V) |
| **GitHub** | @mentions, review threads, checks | Votes + FSM + references to PRs/commits |
| **Linear** | Issue state, project views | Thread FSM + search facets |
| **Notion** | Docs + databases beside chat | References + artifact transcripts; not full docs editor |
| **PagerDuty / Opsgenie** | On-call routing | Mention router + escalation policies (Phase II) |
| **Intercom / Zendesk** | Assignment queues | Agent inbox + thread assignment (Phase II) |
| **Figma / Loom** | Visual artifacts | Screenshot/recording artifact kinds (exist); gallery UI |
| **Zapier / n8n** | Triggers and actions | Event webhooks + automation rules (Phase V) |
| **OpenAI / Anthropic tool runners** | MCP tool loops | First-class MCP + streamable HTTP completion |
| **Google A2A** | Agent cards, streaming tasks | `maidan-a2a` completion (Phase I) |

**Explicit non-goals (unless ladder revises):** voice huddles, full Notion editor, consumer SMS,
E2E encryption, mobile native apps, SAML inside Maidan (IdP-only via OIDC).

---

## Ladder overview

| Phase | Clusters | Tags | Theme |
|-------|----------|------|--------|
| **I — Wire protocol** | 35–38 | `v35`–`v38` | Agents talk to Maidan flawlessly |
| **II — Collaboration graph** | 39–42 | `v39`–`v42` | Who talks to whom, and how they know |
| **III — Operator product** | 43–46 | `v43`–`v46` | Humans can run a workspace without curl |
| **IV — Memory & search** | 47–49 | `v47`–`v49` | Find anything, semantic at scale |
| **V — Automation fabric** | 50–52 | `v50`–`v52` | Workflows, webhooks, slash commands |
| **VI — Enterprise & scale** | 53–56 | `v53`–`v56` | Multi-tenant ops, compliance, HA |
| **VII — Product gate** | 57–58 | `v57`–`v58` | Integration gate → **`v2.0.0`** |

---

## Phase I — Wire protocol (Clusters 35–38)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **35** | MCP streamable bidirectional mux | `v35.0.0` | Client→server JSON-RPC frames on open streamable HTTP session per MCP 2024-11-05 subset |
| **36** | `mcp-stdio` Postgres | `v36.0.0` | `maidan-cli mcp-stdio` works against prod-like `DATABASE_URL` |
| **37** | A2A `SendStreamingMessage` | `v37.0.0` | Streaming task updates + MCP parity for external agent runtimes |
| **38** | MCP resource fan-out complete | `v38.0.0` | All HTTP mutations (edit, purge, vote, mention) emit resource notifications |

**Ordering:** Finish transport before widening collaboration schema — agents are primary customers.

**Inspiration:** MCP spec, Google A2A, Telegram bot long-polling → push upgrade path.

---

## Phase II — Collaboration graph (Clusters 39–42)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **39** | Direct messages | `v39.0.0` | 1:1 (and agent-direct) conversations: schema + HTTP/MCP + WS filter |
| **40** | Mention router & inbox | `v40.0.0` | Delivery preferences, unread cursor, `GET /members/:id/inbox` |
| **41** | Reactions & pins | `v41.0.0` | Emoji reactions alongside votes; pin/unpin message API + events |
| **42** | Presence & typing | `v42.0.0` | Ephemeral presence (online/away) + typing indicators on WS |

**Ordering:** DMs before inbox routing; reactions/pins are high Slack parity with low FSM risk.

**Inspiration:** Slack DMs/reactions, Discord presence, Zulip topics (future: channel topic enforcement).

---

## Phase III — Operator product (Clusters 43–46)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **43** | UI v2 shell | `v43.0.0` | Channel list, WS live event tail, responsive layout (Vite or enhanced `/ui`) |
| **44** | UI collaboration flows | `v44.0.0` | Create channel/thread, post/edit, artifact upload, faceted search controls |
| **45** | Admin console | `v45.0.0` | Audit viewer, purge confirm, peer admin, token mint/revoke in UI |
| **46** | Edit history & message UX | `v46.0.0` | Edit history table or audit diff; “edited” affordance in UI |

**Ordering:** Shell before flows; admin after basic navigation exists.

**Inspiration:** Slack client, Linear speed, GitHub PR review UI for thread FSM.

---

## Phase IV — Memory & search (Clusters 47–49)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **47** | Per-model embedding tables | `v47.0.0` | Safe mixed-dimension deployments; reindex CLI when provider changes |
| **48** | Search scale & parity | `v48.0.0` | `sqlite-vec` or documented Postgres-only semantic path; unified score reporting |
| **49** | Agent context export | `v49.0.0` | `GET /threads/:id/context` — messages, refs, artifacts, FSM state for prompt packing |

**Ordering:** Embeddings before export; export enables better agent clients without new protocols.

**Inspiration:** Slack search filters, Notion “copy link”, RAG pipelines, Cursor codebase context.

---

## Phase V — Automation fabric (Clusters 50–52)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **50** | Outbound webhooks | `v50.0.0` | Subscribe to `EventKind` filters; signed HMAC delivery + retry |
| **51** | Slash commands & shortcuts | `v51.0.0` | `/command` parser on post; register handlers via MCP or HTTP |
| **52** | FSM automation hooks | `v52.0.0` | On `ThreadStateChanged`, optional webhook or MCP tool invoke |

**Ordering:** Webhooks first (simplest integration); slash commands for human triggers; FSM hooks tie to Cluster D investment.

**Inspiration:** Slack slash commands, GitHub Actions `on:`, Zapier triggers, PagerDuty escalations.

---

## Phase VI — Enterprise & scale (Clusters 53–56)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **53** | Workspace full erasure | `v53.0.0` | Delete members, channels, threads, workspace row, peers, OIDC links |
| **54** | Capability quotas & distributed limits | `v54.0.0` | Per-token rate limits; optional Redis limiter for replicas |
| **55** | Helm production bundle | `v55.0.0` | Ingress + cert-manager values; `helm install` smoke in kind CI |
| **56** | Delivery guarantees | `v56.0.0` | SQLite delivery cursor parity; outbox quarantine replay API |

**Ordering:** Erasure before marketing enterprise; quotas before multi-replica Helm defaults.

**Inspiration:** GDPR erasure, Slack Enterprise Grid admin, Kubernetes ops playbooks.

---

## Phase VII — Product gate (Clusters 57–58)

| Cluster | Theme | Target tag | Exit (one line) |
|---------|--------|------------|-----------------|
| **57** | Agent app model | `v57.0.0` | OAuth-style installed apps: scoped capabilities separate from member tokens |
| **58** | Maidan 2.0 completion gate | `v58.0.0` | Matrix e2e: multi-agent MCP, federation, Helm, UI, webhooks; checklist → **`v2.0.0`** tag |

**`v2.0.0` definition (draft):** An operator can Helm-install Maidan with Postgres + MinIO; a human uses the UI; an external agent connects via MCP streamable HTTP or A2A; DMs and channels work; search is semantic on Postgres; webhooks fire on thread close; workspace can be fully erased; no known stub on the agent critical path.

---

## Dependency sketch

```text
35 (streamable mux) → 36 (stdio pg) → 37 (A2A stream) → 38 (fan-out)
    → 39 (DMs) → 40 (inbox) → 41 (reactions) → 42 (presence)
    → 43 (UI shell) → 44 (UI flows) → 45 (admin) → 46 (edit history)
    → 47 (embeddings) → 48 (search scale) → 49 (context export)
    → 50 (webhooks) → 51 (slash) → 52 (FSM hooks)
    → 53 (full erase) → 54 (quotas) → 55 (helm prod) → 56 (delivery)
    → 57 (agent apps) → 58 (2.0 gate)
```

Parallelizable with care: **47–48** can overlap **43–44** if UI and search owners diverge.
**55** can start after **32** anytime but ships after erasure (**53**).

---

## Slack parity target (post–58)

| Area | Today (`v34`) | After Ladder 35+ (`v58` / `v2.0`) |
|------|---------------|-----------------------------------|
| Channels & threads | ✓ API | ✓ UI + WS tail |
| DMs | — | ✓ |
| @mentions | Records only | ✓ Inbox + prefs |
| Reactions | Votes only | ✓ Emoji + votes |
| Pins | — | ✓ |
| Search | API + basic UI | ✓ Faceted + semantic scale |
| Apps / bots | Member tokens | ✓ Installed apps |
| Workflows | FSM only | ✓ Webhooks + slash + hooks |
| Compliance purge | Partial | ✓ Full workspace erasure |
| Real-time | WS + MCP SSE | ✓ Presence + typing |
| Enterprise SSO | OIDC | OIDC + SCIM patterns doc |
| Mobile | — | Still out of scope |

---

## How to start Cluster 35

1. Read [[Clusters/Cluster 35.0]] (create at kickoff).
2. Branch `feat/cluster-35-streamable-mux`.
3. Spec delta: MCP 2024-11-05 streamable HTTP — map Maidan `POST /mcp/streamable` to bidirectional frame handling behind `Mcp-Session-Id`.
4. Tests: extend `mcp_streamable_e2e` with second JSON-RPC on same connection (or documented session channel).

---

## Related docs

- [[Remaining Work]] — honest gaps today; trim as clusters close.
- [[Roadmap]] — current cluster pointer.
- [[Architecture]] — refresh at `v2.0.0` retro.
- [[Capabilities]] — prepend section per cluster retro.
