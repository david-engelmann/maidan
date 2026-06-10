# Roadmap

Maidan ships in clusters. Each cluster ends with a release tag and a
[[Retros/README|retrospective]]. Within a cluster, work is broken into
PRs tracked by the GitHub issues labelled with that cluster.

## Cluster ladder

| Cluster | Theme                                | Target tag |
|---------|--------------------------------------|------------|
| **A**   | Foundation: workspace, schema, /health | `v0.0.1` ✓ |
| **B**   | Routing + event bus + MCP surface    | `v0.1.0` ✓ |
| **C**   | Search + indexing                    | `v0.2.0` ✓ |
| **D**   | FSM-driven thread lifecycle          | `v0.3.0` ✓ |
| **E**   | Artifact substrate (S3, types, refs) | `v0.4.0` ✓ |
| **F**   | Auth, workspaces, capabilities       | `v0.5.0` ✓ |
| **G**   | Agent-to-Agent transport             | `v0.6.0` ✓ |
| **H**   | Web UI + MCP stdio + polish          | `v0.7.0` ✓ |
| **1.0** | Production gates met                 | `v1.0.0` ✓ |

## Cross-cutting tracks

These run in parallel with delivery clusters and do not have their own
tags; they raise the bar each time they ship.

| Track | Theme              | Notes                                   |
|-------|--------------------|-----------------------------------------|
| T     | Telemetry + perf   | OTLP, tracing, latency budgets.         |
| U     | Performance work   | Benchmarks, mutation tests, profiling.  |
| V     | Security + privacy | Threat models, GDPR, secret hygiene.    |
| W     | Documentation      | The vault, runbooks, API docs.          |
| X     | Release engineering| Tags, release notes, signed artifacts.  |

## Current cluster

Clusters A–H and **1.0** are complete (`v1.0.0`). Optional minors **`v1.1.0`**–**`v1.4.0`** are complete.

Post-1.0 work is organized in [Post-1.0.md](Post-1.0.md) and [Tracks/README.md](Tracks/README.md).

Cross-cutting tracks **T, U, V, W, X** are complete.

**Product Ladder 77–101** is **closed on `main`** (operator gate **`maidan-operator-1.0`** at **`v101.0.0`**). Release tag **`v101.0.0`** is the Pi/edge integration point; see [Pi.md](Pi.md).

**Up next:** [Product Ladder 102+](Clusters/Product%20Ladder%20102+.md) — **scale-out, hardening & correctness** (Clusters **102–120**, gate **`maidan-scale-1.0`** at **`v120.0.0`**). Phase **XIX** (102–105, scale-out core) and Phase **XX** (106–110, hot-path hardening) kickoff docs are drafted; Phases **XXI–XXIII** (111–120) are mapped in the ladder.

**Integrators:** use [Integration.md](Integration.md) — not this roadmap.

**Recently closed:** Clusters **93.0**–**101.0** — Operator UI v1, collaboration, operator gate e2e
([Product Ladder 77+.md](Clusters/Product%20Ladder%2077+.md), retros under `docs/Retros/Cluster 93.0.md` … `101.0.md`).

**Recently closed:** Clusters **91.0**–**92.0** — bootstrap strip + `/ui` channel browser at **`v91.0.0`** / **`v92.0.0`**
([[Retros/Cluster 91.0]], [[Retros/Cluster 92.0]]).

**Recently closed:** Clusters **88.0**–**90.0** — Helm profiles, OTLP metrics, SLO alerts at **`v88.0.0`**–**`v90.0.0`**
([[Retros/Cluster 88.0]], [[Retros/Cluster 89.0]], [[Retros/Cluster 90.0]]).

**Recently closed:** Clusters **86.0** and **87.0** — per-model search param + reindex job API at **`v86.0.0`** / **`v87.0.0`**
([[Retros/Cluster 86.0]], [[Retros/Cluster 87.0]]).

**Recently closed:** Cluster **77.0** — HTTP capability map at **`v77.0.0`**
([[Clusters/Cluster 77.0]]).

**Recently closed:** Clusters **71–76** (transport depth + context + ops) at **`v71.0.0`–`v76.0.0`**.

**Recently closed:** **Cluster 70.0** — Vault truth pass at **`v70.0.0`**
([[Retros/Cluster 70.0]]).

**Recently closed:** **Cluster 69.0** — Capabilities matrix complete at **`v69.0.0`**
([[Retros/Cluster 69.0]]).

**Recently closed:** **Cluster 68.0** — Automation delivery guarantees at **`v68.0.0`**
([[Retros/Cluster 68.0]]).

**Recently closed:** Product Ladder **59+** at **`v67.0.0`** ([[Clusters/Product Ladder 59+]],
[[Agent Integration]]).

**Recently closed:** **Cluster 67.0** — Workspace context packages at **`v67.0.0`**.

**Recently closed:** **Cluster 58.0** — Maidan 2.0 completion gate at **`v58.0.0`**
([[Retros/Cluster 58.0]]).

**Recently closed:** **Cluster 57.0** — Agent app model at **`v57.0.0`** ([[Retros/Cluster 57.0]]).

**Recently closed:** **Cluster 56.0** — Delivery guarantees at **`v56.0.0`** ([[Retros/Cluster 56.0]]).

**Recently closed:** **Cluster 55.0** — Helm production bundle at **`v55.0.0`** ([[Retros/Cluster 55.0]]).

**Recently closed:** **Cluster 54.0** — Capability quotas at **`v54.0.0`** ([[Retros/Cluster 54.0]]).

**Recently closed:** **Cluster 53.0** — Workspace full erasure at **`v53.0.0`** ([[Retros/Cluster 53.0]]).

**Recently closed:** **Cluster 52.0** — FSM automation hooks at **`v52.0.0`** ([[Retros/Cluster 52.0]]).

**Recently closed:** **Cluster 51.0** — Slash commands at **`v51.0.0`** ([[Retros/Cluster 51.0]]).

**Recently closed:** **Cluster 49.0** — Agent context export at **`v49.0.0`** ([[Retros/Cluster 49.0]]).

**Recently closed:** **Cluster 38.0** — MCP resource fan-out complete at **`v38.0.0`** ([[Retros/Cluster 38.0]]).

**Recently closed:** **Cluster 37.0** — A2A `SendStreamingMessage` at **`v37.0.0`** ([[Retros/Cluster 37.0]]).

**Recently closed:** **Cluster 36.0** — `mcp-stdio` Postgres at **`v36.0.0`** ([[Retros/Cluster 36.0]]).

**Recently closed:** **Cluster 35.0** — MCP streamable bidirectional mux at **`v35.0.0`** ([[Retros/Cluster 35.0]]).

**Recently closed:** Product Ladder **30–34** at **`v34.0.0`** ([[Retros/Product Ladder 30-34]], [[Clusters/Product Ladder 30-34]]).

**Recently closed:** **Cluster 32.0** — Helm umbrella at **`v32.0.0`** ([[Retros/Cluster 32.0]]).

**Recently closed:** **Cluster 31.0** — workspace artifact purge at **`v31.0.0`** ([[Retros/Cluster 31.0]]).

**Recently closed:** **Cluster 30.0** — rate limits at **`v30.0.0`** ([[Retros/Cluster 30.0]]).

**Recently closed:** **Cluster 29.0** — message edit at **`v29.0.0`** ([[Retros/Cluster 29.0]]).

**Recently closed:** **Cluster 28.0** — privacy complete at **`v28.0.0`** ([[Retros/Cluster 28.0]]).

**Recently closed:** Product Ladder **17–27** at **`v27.0.0`**
([[Retros/Cluster 27.0]], PR #198); tags **`v23.0.0`–`v27.0.0`** documented in
CHANGELOG (GitHub Release cut at **`v27.0.0`**).

**Before that:** Product Ladder integration ([[Clusters/Product Ladder 17-27]]);
**`v22.0.0`** — capabilities hardening ([[Retros/Cluster 22.0]]).
**Before that:** **`v21.0.0`** — A2A agent transport ([[Retros/Cluster 21.0]]).
**Before that:** **`v20.0.0`** — message router ([[Retros/Cluster 20.0]]).
**Before that:** **`v19.0.0`** — S3 multipart artifacts ([[Retros/Cluster 19.0]]).
**Before that:** **`v18.0.0`** — SQLite semantic search ([[Retros/Cluster 18.0]]).
**Before that:** **`v17.0.0`** — MCP resource fan-out ([[Retros/Cluster 17.0]]).
**Before that:** **`v16.0.0`** — MCP HTTP resource notifications ([[Retros/Cluster 16.0]]).
**Before that:** **`v15.0.0`** — MCP stdio resource subscribe ([[Retros/Cluster 15.0]]).
**Before that:** **`v14.0.0`** — SQLite outbox ([[Retros/Cluster 14.0]]).
**Before that:** **`v13.0.0`** — delivery ledger ([[Retros/Cluster 13.0]]).
**Before that:** **`v12.0.0`** — outbox relay hardening ([[Retros/Cluster 12.0]]).
**Before that:** **`v11.0.0`** — coverage 11% ([[Retros/Cluster 11.0]]).
**Before that:** **`v10.0.0`** — Postgres transactional outbox ([[Retros/Cluster 10.0]]).
**Before that:** **`v9.0.0`** — coverage depth ([[Retros/Cluster 9.0]]).
**Before that:** **`v8.0.0`** — bus hydrate observability ([[Retros/Cluster 8.0]]).
**Before that:** **`v7.0.0`** — bus pointer delivery ([[Retros/Cluster 7.0]]).
**Before that:** **`v6.0.0`** — delivery reliability ([[Retros/Cluster 6.0]]).
**Before that:** **`v5.0.0`** — coverage & search quality ([[Retros/Cluster 5.0]]).
**Before that:** **`v4.0.0`** — subscriber continuity ([[Retros/Cluster 4.0]]).
**Before that:** **`v3.0.0`** — search & subscriber depth ([[Retros/Cluster 3.0]]).
**Before that:** **`v2.1.0`** — OIDC operator hardening ([[Retros/Cluster 2.1]]).

**Also on deck:** ad-hoc reliability/search backlog in [[Open Work]].

## Closing a cluster

Each cluster closes with a dedicated retro PR that:

- Creates [[Retros/README|the retro note]] for that cluster.
- Updates [[Capabilities]].
- Updates the root `CHANGELOG.md`.
- Cuts the release tag.

This pattern is mandatory; tags are never cut without a retro.
