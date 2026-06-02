# Open work

Aggregate of deferred items across retros plus standing risks — the
“if I had two hours” backlog. For exhaustive partials and Slack parity,
see [[Remaining Work]].

Updated at each cluster retro. **Baseline:** code on `main` through **`v69.0.0`**.

## Standing risks (still open)

- **At-most-once event bus** — transactional outbox (**10**), quarantine (**12**), HTTP outbox replay (**56**); NOTIFY duplicates still possible.
- **Bootstrap / `AUTH_DISABLED`** — high-impact misconfiguration.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener** — best-effort recovery; `/health/ready` reflects errors.
- **Coverage floor 11%** — incremental uplift opportunistic.
- **SQLite semantic search** — brute-force cosine; no HNSW on SQLite.
- **`hash-v1` default** — not semantic until a real provider is configured.

## Shipped (reference)

| Ladder / tag | Highlights |
|--------------|------------|
| **17–27** | MCP fan-out, SQLite semantic, Helm server, purge, streamable subset |
| **35–58** | `maidan-2.0` product gate — DMs, webhooks, slash, FSM, erase, quotas, completion e2e |
| **59–67** | [[Agent Integration]], streamable TTL, A2A card, outbox ops, app OAuth, context |
| **68–69** | Automation delivery DLQ; MCP capability map + matrix e2e |

**Still manual:** Sigstore/cosign release artifacts ([[Operations]]).

## Agent substrate ladder (68+)

Active plan: [[Clusters/Product Ladder 68+]] → **`maidan-agent-1.0`** at **76**.

| Tag | Theme |
|-----|--------|
| **`v68.0.0`** | Slash/FSM HTTP delivery ledger — [[Retros/Cluster 68.0]] |
| **`v69.0.0`** | MCP capability map + CI matrix — [[Retros/Cluster 69.0]] |
| **`v70.0.0`** | Vault truth — [[Retros/Cluster 70.0]] |

**Ladder 68–76 closed** at **`v76.0.0`** ([[Retros/Cluster 76.0]]). Next work: human-product backlog ([[Remaining Work]]) or external integrator deployments.

## Still deferred (no separate owner)

| What | Notes |
|------|-------|
| Full MCP streamable 2024-11-05 session | **73** |
| Persisted A2A task push | **72** |
| MCP context export tools | **74** |
| OpenAPI ↔ capability CI for all routes | Beyond **69** sample HTTP contract |
| `sqlite-vec` / per-model embedding tables | **75** / Open Work standing risks |
| OTLP dashboards + agent gate e2e | **76** |
| Multi-region active-active | Out of scope |

## Known state

- **Latest tag on `main`:** **`v69.0.0`** (cut **`v70.0.0`** after Cluster 70 retro merges).
- **Active cluster:** **71** after **70** closes ([[Clusters/Product Ladder 68+]]).
- **Integrators:** start at [[Agent Integration]] and `contracts/`.

## How to read this file

- **[[Remaining Work]]** — partial implementations + Slack matrix.
- **[[Roadmap]]** — cluster pointer and historical closes.
- Retro PRs are the right time to add or remove deferrals.
