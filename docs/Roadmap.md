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

**Product Ladder 77–101** is **closed on `main`**; the operator gate **`maidan-operator-1.0`** is tagged at **`v101.0.0`** (the Pi/edge integration point, see [Pi.md](Pi.md)). Clusters 93–101 shipped as one batch (PR #264) released as `v101.0.0`, so there are no separate `v93.0.0`–`v100.0.0` tags.

**Product Ladder 102+ is COMPLETE.** [Product Ladder 102+](Clusters/Product%20Ladder%20102+.md) — **scale-out, hardening & correctness** — closed across Phases **XIX (scale-out core, 102–105)**, **XX (hot-path hardening, 106–110)**, **XXI (correctness & coverage, 111–115)**, **XXII (search & indexer at scale, 116–118)**, and **XXIII (supply chain & scale gate, 119–120)**, tags **`v102.0.0`–`v120.0.0`**. The **`maidan-scale-1.0`** product gate is tagged at **`v120.0.0`** ([[Gates/maidan-scale-1.0]]), alongside `maidan-operator-1.0` (`v101`), `maidan-agent-1.0` (`v76`), and `maidan-2.0` (`v58`) — all four gate tags are cut. No further ladder cluster is defined past 120; future work is post-gate human-product and the cross-cutting tracks ([[Open Work]], [[Remaining Work]]).

**Post-gate hardening (Phase XXIV):** with the ladder closed, work continues opportunistically from [[Open Work]] / [[Remaining Work]], tagged on the same `vX.0.0` ladder but **without** new gate tags. Cluster **121.0** (`v121.0.0`) opened it (OpenAPI-wide capability map in CI + scale-out SLO coverage); Cluster **122.0** (`v122.0.0`) added promtool execution of the SLO alert rules; Cluster **123.0** (`v123.0.0`) proved OTLP export end-to-end against a real collector; Cluster **124.0** (`v124.0.0`) consolidated the rule validators and promoted the alert-rules + otlp-smoke jobs to required checks (8 total); Cluster **125.0** (`v125.0.0`) added opt-in at-least-once event delivery; Cluster **126.0** (`v126.0.0`) extended it to the MCP SSE transport; Cluster **127.0** (`v127.0.0`) reconciled the backlog; **128.0** (`v128.0.0`) hardened A2A delivery; **129.0** (`v129.0.0`) bounded buffers + error visibility; **130.0** (`v130.0.0`) lifted observability/MCP test coverage; **131.0** (`v131.0.0`) closed delivery-unification; **132.0** (`v132.0.0`) shipped the global admin audit query API (completing the 127–132 sweep). A **UI track** then began: **133.0** (`v133.0.0`) repaired the broken `/ui` write path + added a JS guard; **134.0** (`v134.0.0`) added message reactions; **135.0** (`v135.0.0`) added message pins; **136.0** (`v136.0.0`) added group DMs (new tab). Remaining: 137 operator console.

**Integrators:** use [Integration.md](Integration.md) — not this roadmap.

**Recently closed:** Cluster **136.0** — group DMs in the `/ui` console (open/list/read/post over `/ui/api`, new tab) at **`v136.0.0`**
([[Retros/Cluster 136.0]]).

**Recently closed:** Cluster **135.0** — pin/unpin in the `/ui` thread view (toggle over `/ui/api`) at **`v135.0.0`**
([[Retros/Cluster 135.0]]).

**Recently closed:** Cluster **134.0** — emoji reactions in the `/ui` console (chips/quick-add/toggle over `/ui/api`) at **`v134.0.0`**
([[Retros/Cluster 134.0]]).

**Recently closed:** Cluster **133.0** — `/ui` write-path repair (4 undefined JS refs) + `ui_js_contract` guard, at **`v133.0.0`**
([[Retros/Cluster 133.0]]).

**Recently closed:** Cluster **132.0** — global cross-workspace admin audit query API (`GET /operator/audit`, gated by `audit:read-global`) at **`v132.0.0`**
([[Retros/Cluster 132.0]]).

**Recently closed:** Cluster **131.0** — delivery-unification verification-close (signing/backoff + operator API already unified; storage intentionally separate; risky migration declined) at **`v131.0.0`**
([[Retros/Cluster 131.0]]).

**Recently closed:** Cluster **130.0** — test-coverage uplift (observability env-parsing pure parsers + MCP prompts integrity) at **`v130.0.0`**
([[Retros/Cluster 130.0]]).

**Recently closed:** Cluster **129.0** — hardening: bounded MCP streamable buffer, outbox quarantine-failure visibility, `unreachable!()` → typed errors, at **`v129.0.0`**
([[Retros/Cluster 129.0]]).

**Recently closed:** Cluster **128.0** — A2A delivery robustness (client timeouts; push retry/backoff + `maidan_a2a_push_total`; SSE error visibility) at **`v128.0.0`**
([[Retros/Cluster 128.0]]).

**Recently closed:** Cluster **127.0** — backlog reconciliation (corrected ~11 phantom entries + the stale `Open Work` tail against code at v126) at **`v127.0.0`**
([[Retros/Cluster 127.0]]).

**Recently closed:** Cluster **126.0** — MCP SSE at-least-once parity (`at_least_once` on `/mcp/stream`, reusing the reconcile loop) at **`v126.0.0`**
([[Retros/Cluster 126.0]]).

**Recently closed:** Cluster **125.0** — at-least-once event delivery (opt-in `at_least_once` subscribe: cursor-driven reconcile over a stability horizon; closes the silent out-of-order gap) at **`v125.0.0`**
([[Retros/Cluster 125.0]]).

**Recently closed:** Cluster **124.0** — CI/observability loose ends (one SLO-rule validator; `promtool (alert rules)` + `otlp smoke` promoted to required, 8 checks total) at **`v124.0.0`**
([[Retros/Cluster 124.0]]).

**Recently closed:** Cluster **123.0** — OTLP end-to-end collector smoke (server pushes traces + metrics to a real OpenTelemetry Collector; CI asserts delivery) at **`v123.0.0`**
([[Retros/Cluster 123.0]]).

**Recently closed:** Cluster **122.0** — execute the SLO alert rules in CI with promtool (caught + fixed a `$value`-rendering bug; corrected the OTLP-export status) at **`v122.0.0`**
([[Retros/Cluster 122.0]]).

**Recently closed:** Cluster **121.0** — observability & contract completeness (every OpenAPI op classified in CI; SLO alerts/dashboard extended to the Cluster 116 indexer metrics) at **`v121.0.0`**, opening Phase XXIV (post-gate hardening)
([[Retros/Cluster 121.0]]).

**Recently closed:** Cluster **120.0** — scale product gate at **`v120.0.0`** / **`maidan-scale-1.0`**, closing Phase XXIII and the 102+ ladder
([[Retros/Cluster 120.0]]).

**Recently closed:** Cluster **119.0** — dependency dedupe & currency (thiserror 2, `deny.toml` duplicate-major gate, edition-2024 eval) at **`v119.0.0`**, opening Phase XXIII
([[Retros/Cluster 119.0]]).

**Recently closed:** Cluster **118.0** — hybrid lexical+semantic relevance + eval harness at **`v118.0.0`**, closing Phase XXII
([[Retros/Cluster 118.0]]).

**Recently closed:** Cluster **117.0** — pluggable production provider (dimension auto-detect + boot-time model registration) at **`v117.0.0`**
([[Retros/Cluster 117.0]]).

**Recently closed:** Cluster **116.0** — batch embedding pipeline (bounded backpressure + chunked backfill) at **`v116.0.0`**, opening Phase XXII
([[Retros/Cluster 116.0]]).

**Recently closed:** Cluster **115.0** — module split + `unwrap()` purge at **`v115.0.0`**, closing Phase XXI
([[Retros/Cluster 115.0]]).

**Recently closed:** Cluster **114.0** — coverage uplift + envelope fuzz (full-suite gate at 40%) at **`v114.0.0`**
([[Retros/Cluster 114.0]]).

**Recently closed:** Cluster **113.0** — backend parity harness at **`v113.0.0`**
([[Retros/Cluster 113.0]]).

**Recently closed:** Cluster **112.0** — FSM property tests at **`v112.0.0`**
([[Retros/Cluster 112.0]]).

**Recently closed:** Cluster **111.0** — `maidan-auth` test suite at **`v111.0.0`**, opening Phase XXI
([[Retros/Cluster 111.0]]).

**Recently closed:** Cluster **110.0** — per-workspace fairness at **`v110.0.0`**, closing Phase XX
([[Retros/Cluster 110.0]]).

**Recently closed:** Cluster **109.0** — ANN index tuning + search bench at **`v109.0.0`**
([[Retros/Cluster 109.0]]).

**Recently closed:** Cluster **108.0** — adaptive outbox relay (drain-until-empty + idle backoff + enqueue nudge) at **`v108.0.0`**
([[Retros/Cluster 108.0]]).

**Recently closed:** Cluster **107.0** — configurable DB pool & timeouts at **`v107.0.0`**
([[Retros/Cluster 107.0]]).

**Recently closed:** Cluster **106.0** — bulk context reads (N+1 elimination) at **`v106.0.0`**
([[Retros/Cluster 106.0]]).

**Recently closed:** Cluster **105.0** — multi-replica scale-out smoke at **`v105.0.0`**, closing Phase XIX
([[Retros/Cluster 105.0]]).

**Recently closed:** Cluster **104.0** — durable ephemeral state (OAuth codes + reindex jobs) at **`v104.0.0`**
([[Retros/Cluster 104.0]]).

**Recently closed:** Cluster **103.0** — distributed presence & roster at **`v103.0.0`**
([[Retros/Cluster 103.0]]).

**Recently closed:** Cluster **102.0** — cross-replica MCP resource notifications at **`v102.0.0`**
([[Retros/Cluster 102.0]]); first cluster of [Product Ladder 102+](Clusters/Product%20Ladder%20102+.md).

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
