# Product Ladder 68+ — Agent substrate completion

**North star:** Close every gap left from [[Clusters/Product Ladder 59+]] so external agents can integrate against a **complete, tested, documented** contract — without human-Slack chrome (UI v3, huddles, org hierarchy).

**Predecessor:** [[Clusters/Product Ladder 59+]] closed at **`v67.0.0`** ([[Agent Integration]]).

**Versioning:** One cluster → one tag (`v68.0.0` …). Product gate after Cluster **76**: tag **`maidan-agent-1.0`** at the same commit as **`v76.0.0`** (semver **`v1.0.0`** is Cluster 1.0 production gates — do not reuse).

---

## Honest gap register (59–67 vs kickoff)

| Cluster | Kickoff promise | Shipped at tag | Deferred to |
|---------|-----------------|----------------|-------------|
| **59** | Charter + **vault truth** (Architecture, Remaining Work, README) | Contract golden files + [[Agent Integration]] | **70** |
| **60** | Full MCP 2024-11-05 streamable subset | TTL + `DELETE /mcp/streamable` | **73** |
| **61** | A2A external runtime (push + discovery) | Agent card + in-memory push config RPC | **72** |
| **62** | Subscribe v2 + operator outbox ops | `schema_version` + list quarantined | **71** (event/MCP parity) |
| **63** | **Every** MCP tool + critical HTTP in capability matrix | One MCP denial e2e | **69** |
| **64** | MCP quotas | Per-token `tools/call` quotas | — (done) |
| **65–67** | OAuth, well-known, context packages | As tagged | **74** extends context via MCP |

HTTP targets for slash commands and FSM hooks (Clusters **51**, **52**) now have **reliable delivery** at **`v68.0.0`** (ledger, retries, DLQ, replay). Webhooks already had `maidan_webhook_deliveries`; cluster **68** did not merge those paths.

---

## Ladder overview

| Phase | Clusters | Tags | Theme |
|-------|----------|------|--------|
| **XI — Close 59–67 debt** | 68–70 | `v68`–`v70` | Delivery, auth matrix, docs truth |
| **XII — Transport depth** | 71–73 | `v71`–`v73` | Events, A2A tasks, MCP streamable |
| **XIII — Memory & ops** | 74–76 | `v74`–`v76` | Context export, semantic scale, SLOs |

---

## Phase XI — Close 59–67 debt (Clusters 68–70)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **68** | Automation delivery guarantees | `v68.0.0` ✓ | Slash + FSM HTTP use **`maidan_automation_deliveries`** (retries, DLQ, replay); webhooks unchanged on **`maidan_webhook_deliveries`** |
| **69** | Capabilities matrix complete | `v69.0.0` ✓ | MCP catalog + capability map in CI; table-driven deny/allow gate per tool; HTTP sample contract |
| **70** | Vault truth pass | `v70.0.0` ✓ | [[Architecture]], [[Remaining Work]], [[Open Work]], root `README.md` reflect **`v69.0.0`** reality; stale “not implemented” rows removed |

**Ordering:** **70** can start in parallel with **68** (docs-only PRs). **69** should follow **70** so the capability map lists routes that exist. **68** is independent of **69**.

**Inspiration:** Slack retry logs, GitHub Actions delivery, Cluster **12** outbox quarantine.

---

## Phase XII — Transport depth (Clusters 71–73)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **71** | Event & subscribe contract v2 | `v71.0.0` ✓ | WS filter schema + EventKind forward-compat + MCP notification checklist CI |
| **72** | A2A task streaming | `v72.0.0` ✓ | Persisted push config + `SubscribeToTask` SSE |
| **73** | MCP streamable complete | `v73.0.0` ✓ | Session delete e2e + [[Agent Integration]] lifecycle |

**Ordering:** **71** before **72** (shared event semantics). **73** parallelizable after **70**.

---

## Phase XIII — Memory & ops (Clusters 74–76)

| Cluster | Theme | Tag | Exit (one line) |
|---------|--------|-----|-----------------|
| **74** | Context export parity | `v74.0.0` ✓ | MCP `get_thread_context` / `get_workspace_context` |
| **75** | Semantic scale | `v75.0.0` ✓ | `maidan reindex-embeddings` runbook + embedding provider guidance |
| **76** | Agent observability | `v76.0.0` ✓ | Agent metrics runbook + **`maidan-agent-1.0`** gate e2e |

**Ordering:** **74** after **67** baseline. **75** needs store/search only. **76** last (depends on **68** DLQ metrics).

---

## Dependency sketch

```text
70 (docs) ─────────────────────────────┐
68 (automation DLQ) ───────────────────┼→ 76 (SLOs + maidan-agent-1.0)
69 (capabilities) ← 70                 │
71 (event contract) → 72 (A2A tasks)   │
73 (MCP streamable) ← 70               │
74 (context MCP) ← 67                │
75 (semantic) ← 47–48                │
```

**Parallelizable after 70:** 68 + 71 + 73 + 75.

---

## Per-cluster kickoff docs

Create `docs/Clusters/Cluster N.0.md` when starting each cluster (copy [[Clusters/Cluster 58.0]] template). This file is the **epic map** only.

| Cluster | Kickoff doc |
|---------|-------------|
| **68** | [[Clusters/Cluster 68.0]] |
| **69–76** | Create at kickoff |

---

## Explicitly not in 68–76 (human-product / post-gate)

| Item | Track |
|------|--------|
| React SPA / full `/ui` channel browser | Human-product backlog ([[Remaining Work]] §4) |
| Group DMs, huddles, org hierarchy | Same |
| Multi-region active-active | [[Open Work]] |
| SAML/SCIM in Maidan | [[OIDC]] non-goals |
| `sqlite-vec` / HNSW on SQLite | Optional post-**75** spike |
| Bootstrap routes compile-time removal | Optional **77** if threat model demands |

---

## Product gate: `maidan-agent-1.0`

At **`v76.0.0`**, an external agent integrator can:

1. Discover endpoints via well-known + agent card (**66–66**).
2. Onboard via app OAuth (**65**).
3. Call MCP/A2A/WS with **no surprise 403s** (**69**).
4. Rely on signed automation callbacks with retries (**68**).
5. Export context for LLM prompts (**67**, **74**).
6. Operate with documented SLOs and dashboards (**76**).

---

## Continuation

Post-gate work continues in [[Clusters/Product Ladder 77+]] (Clusters **77–101**, gate **`maidan-operator-1.0`**).

---

## References

- [[Agent Integration]], [[Clusters/Product Ladder 59+]], [[Retros/Cluster 52.0]] (FSM hook deferrals)
- [[Clusters/Cluster 22.0]], [[Clusters/Cluster 56.0]] (outbox replay baseline)
- [[Clusters/Product Ladder 77+]], [[Remaining Work]], [[Open Work]], [[Roadmap]]
