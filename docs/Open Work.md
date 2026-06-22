# Open work

Aggregate of deferred items across retros plus standing risks — the
“if I had two hours” backlog. For exhaustive partials and Slack parity,
see [[Remaining Work]].

Updated at each cluster retro. **Baseline:** code on `main` at **`v120.0.0`** (Product Ladder 102+ complete; scale gate `maidan-scale-1.0`).

## Standing risks (still open)

- **At-most-once event bus** — transactional outbox (**10**), quarantine (**12**), HTTP outbox replay (**56**); NOTIFY duplicates still possible.
- **Bootstrap / `AUTH_DISABLED`** — high-impact misconfiguration.
- **Indexer staleness** — opt-in `INDEXER_STALE_SECS`.
- **PostgresBus listener** — best-effort recovery; `/health/ready` reflects errors.
- **SQLite semantic search** — brute-force cosine; no HNSW on SQLite.
- **`hash-v1` default** — `openai-compatible` provider (v117) gives real semantics; `hash-v1` is the offline/dev default, not semantically meaningful.
- **`rsa` advisory `RUSTSEC-2023-0071`** — ignored (RS256 id_token verify via openidconnect v4; no fixed `rsa`); clears on openidconnect v5 (unreleased). See [Dependencies.md](Dependencies.md).
- **No `v93`–`v100` tags** — clusters 93–101 shipped as one batch (PR #264), released as `v101.0.0`; not a backlog. All four gate tags (incl. `maidan-operator-1.0`) are cut.

## Shipped (reference)

| Ladder / tag | Highlights |
|--------------|------------|
| **17–27** | MCP fan-out, SQLite semantic, Helm server, purge, streamable subset |
| **35–58** | `maidan-2.0` product gate — DMs, webhooks, slash, FSM, erase, quotas, completion e2e |
| **59–67** | [[Agent Integration]], streamable TTL, A2A card, outbox ops, app OAuth, context |
| **68–76** | Automation DLQ, capability map, vault truth, A2A subscribe, MCP context, agent gate — [[Retros/Cluster 76.0]] |

**Still manual:** Sigstore/cosign release artifacts ([[Operations]]).

## Agent substrate ladder (68+)

**Closed** at **`v76.0.0`** and **`maidan-agent-1.0`**.

**Next ladder:** [[Clusters/Product Ladder 77+]] (**77–101**, gate **`maidan-operator-1.0`**). Opportunistic human-product items remain in [[Remaining Work]] §4.

## Still deferred (no separate owner)

| What | Notes |
|------|-------|
| Full MCP streamable 2024-11-05 bidirectional mux | Subset shipped in **73**; spec-complete session still open |
| OpenAPI ↔ capability map for every HTTP route | **69** shipped sample contract + full MCP matrix |
| Hosted OTLP / Grafana dashboards | **76** shipped metrics runbook only |
| `sqlite-vec` / per-model embedding tables | Standing risk |
| Multi-region active-active | Out of scope |
| Unify webhooks + automation delivery queues | **68** retro deferral |

## Known state

- **Latest tag:** **`v76.0.0`** / **`maidan-agent-1.0`**.
- **Active cluster:** **78** ([[Clusters/Product Ladder 77+]]).
- **Integrators:** start at [[Agent Integration]] and `contracts/`.

## How to read this file

- **[[Remaining Work]]** — partial implementations + Slack matrix.
- **[[Roadmap]]** — cluster pointer and historical closes.
- Retro PRs are the right time to add or remove deferrals.
