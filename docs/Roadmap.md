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

Clusters A–H and **1.0** are complete (`v1.0.0`). Optional minors **`v1.1.0`**,
**`v1.2.0`**, **`v1.3.0`**, and **`v1.4.0`** are complete — see corresponding
minor retros.

Post-1.0 work is organized in [[Post-1.0]] and [[Tracks/README]].

Cross-cutting tracks **T, U, V, W, X** are complete (see [[Post-1.0]]).

**Active:** **Cluster 38.0** — MCP resource fan-out complete ([[Clusters/Product Ladder 35+]] Phase I).

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
