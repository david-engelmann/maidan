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

**Post-gate hardening (Phase XXIV):** with the ladder closed, work continues opportunistically from [[Open Work]] / [[Remaining Work]], tagged on the same `vX.0.0` ladder but **without** new gate tags. Cluster **121.0** (`v121.0.0`) opened it (OpenAPI-wide capability map in CI + scale-out SLO coverage); Cluster **122.0** (`v122.0.0`) added promtool execution of the SLO alert rules; Cluster **123.0** (`v123.0.0`) proved OTLP export end-to-end against a real collector; Cluster **124.0** (`v124.0.0`) consolidated the rule validators and promoted the alert-rules + otlp-smoke jobs to required checks (8 total); Cluster **125.0** (`v125.0.0`) added opt-in at-least-once event delivery; Cluster **126.0** (`v126.0.0`) extended it to the MCP SSE transport; Cluster **127.0** (`v127.0.0`) reconciled the backlog; **128.0** (`v128.0.0`) hardened A2A delivery; **129.0** (`v129.0.0`) bounded buffers + error visibility; **130.0** (`v130.0.0`) lifted observability/MCP test coverage; **131.0** (`v131.0.0`) closed delivery-unification; **132.0** (`v132.0.0`) shipped the global admin audit query API (completing the 127–132 sweep). A **UI track** then began: **133.0** (`v133.0.0`) repaired the broken `/ui` write path + added a JS guard; **134.0** (`v134.0.0`) added message reactions; **135.0** (`v135.0.0`) added message pins; **136.0** (`v136.0.0`) added group DMs (new tab); **137.0** (`v137.0.0`) added a deliveries & DLQ operator view (list + filter + replay); **138.0** (`v138.0.0`) completed the "Operator" tab with global-audit + reindex controls (operator-console arc 137–138 complete); **139.0** (`v139.0.0`) added 1:1 direct messages (new "DMs" tab, the parallel to group DMs); **140.0** (`v140.0.0`) added a workspace presence roster (new "Presence" tab, rendering the WS `presence_snapshot` frames). **141.0** (`v141.0.0`) fixed the published mdBook site — its sidebar had ~20 dead links (mdBook silently skipped the `../docs/*` sources); a build-time staging step now publishes all 21 SUMMARY pages, plus a landing quickstart and a helpful 404. **142.0** (`v142.0.0`) added the slash-command registry (new "Slash" tab: register/list/revoke), surfacing the last unsurfaced backend collaboration feature. The `/ui` now covers the full backend surface; remaining work is polish / new product rather than catch-up. **143.0** (`v143.0.0`) began UI polish: richer message rendering (timestamps + inline slash-command results), surfacing payload data the thread view didn't show. **144.0** (`v144.0.0`) added a docs dead-link gate (`mdbook-linkcheck`, `warning-policy = error`) — the 141 follow-up — which surfaced + fixed 35 latent broken published links (space-files hyphenated, out-of-set links GitHub-rewritten) and reconciled the backlog docs (132 audit API + 134–143 UI track). The docs pipeline now self-guards against broken-nav regressions. An **MCP streamable spec-completeness arc (145–148)** then began: **145.0** (`v145.0.0`) landed the JSON-RPC/lifecycle conformance basics — `initialize` protocol-version negotiation, `MCP-Protocol-Version` header validation, JSON-RPC batching + notifications on `POST /mcp`; the streamable-transport gaps (GET SSE + `Accept` negotiation, resumability, server→client requests) follow in 146–148. **146.0** (`v146.0.0`) added `GET /mcp/streamable` (server→client SSE stream) + `Accept`-based JSON/SSE content negotiation on the POST; **147.0** (`v147.0.0`) added resumability — SSE `id:` on session frames + `Last-Event-ID` reconnect replay (bounded per-session log; the session now survives a dropped POST leg). **148.0** (`v148.0.0`) concluded the arc with server→client requests (sampling / roots / elicitation via `request_client`, capability-gated + correlated) + per-session client-capability tracking. The **MCP streamable spec-completeness backlog item is closed** — no open backend capability gaps remain. After next-arc research (UI polish, missing features, token efficiency, `request_client`), an **MCP-agent-surface arc** began: **149.0** (`v149.0.0`) added MCP inbox/mention tools (`list_mentions`/`get_inbox`/`mark_inbox_read`) so an MCP-only agent can discover it was @mentioned; **150.0** (`v150.0.0`) added thread/member/kind filters to `/mcp/stream` (await my mention). The MCP-agent-surface pair is complete. A **token-efficiency** cluster followed: **151.0** (`v151.0.0`) made `get_thread_context` edits lean by default (`{id, editor, edited_at}`; opt-in `include_edits=true` for full bodies) — edit bodies were the largest token cost in a context pack — and clamped `list_messages` to `1..=500`. **152.0** (`v152.0.0`) brought the same lean-edits default to the **REST** context pack (`GET /threads/:id/context` + `/workspaces/:wid/context`, via `MessageEditView` with optional bodies + `include_edits` query param) and added `snippet_only=true` to `GET …/search` (drops full bodies; semantic hits get a truncated snippet). The token-efficiency lane now covers both context surfaces + search. **153.0** (`v153.0.0`) shipped lane 2 — a **live-updating `/ui` thread view**: WS message/reaction/pin frames for the open thread now refresh the message list (debounced) instead of only landing as Events-tab log lines. Lane 3 (`request_client`) then began: **154.0** (`v154.0.0`) fixed **GET-stream delivery** — server→client requests (sampling/roots/elicitation) now ride a per-session broadcast merged into the spec-canonical `GET /mcp/streamable` stream (they previously reached only a POST-leg SSE holder). **155.0** (`v155.0.0`) closed it with a **real caller**: the sampling-backed **`summarize_thread`** tool threads the streamable session id through `handle_in_session`→`dispatch`→`tools_call` and issues a server→client `sampling/createMessage` over the GET stream. **The three-lane next-arc plan is complete** (token efficiency 151+152, live UI 153, request_client 154+155). A 5-agent research sweep (feature-gaps, performance, CI/CD, token, production-readiness) then set the next program — four arcs to run in order toward enterprise production-readiness: **(1) hardening** (quick-wins → channel/thread RBAC, the #1 finding), **(2) perf + CI/CD**, **(3) agentic features** (structured content, backpressure, HITL approvals, task handoff), **(4) token round 3**. Arc 1 began: **156.0** (`v156.0.0`) shipped production-safety defaults — SIGTERM graceful shutdown (k8s/systemd drain) + a default 30 s `statement_timeout`. **157.0** (`v157.0.0`) made `AUTH_DISABLED` fail-closed — it now requires the explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack (and never in production), closing the silent-open-door risk; coordinated across the compose/helm CI manifests. **158.0** (`v158.0.0`) added keyless cosign signatures to the container images (server + postgres, by digest), closing the unsigned-images gap. Arc-1 hardening quick-wins are done; the arc's **flagship channel/thread RBAC** then began (the #1 finding — authz is workspace-flat), planned as three clusters (membership model → enforcement → management API; Postgres RLS deferred). **159.0** (`v159.0.0`) landed part A: the `channel_members` model + store + migration (both backends), additive with no enforcement. **160.0** (`v160.0.0`) landed part B: `ensure_channel_access` enforced on every REST content route + search + workspace-context (private channels need a membership row; public + `__dm__` unchanged; creator auto-added on private create) — closing the workspace-flat read/write vuln on REST. **161.0** (`v161.0.0`) landed part C: MCP point-access enforcement (a pre-dispatch gate on the content tools + `resources/read`), closing the MCP read/write path into private channels. **162.0** (`v162.0.0`) filtered the MCP aggregate reads (search / list-channels / workspace-context), closing the channel-content vuln on REST + MCP. **163.0** (`v163.0.0`) verified WS/MCP subscribe grants against membership, closing the private-channel event leak. **164.0** (`v164.0.0`) added the `channel:admin` capability + `/channels/:cid/members` REST + MCP membership API, making private channels operational. **165.0** (`v165.0.0`) guarded `reference.rs` (REST + MCP `add_reference`) via the entity→channel access helpers, **completing the channel/thread RBAC arc (159–165)**. Arc 1 (enterprise hardening) is done; **arc 2 (perf + CI/CD)** began: **166.0** (`v166.0.0`) fixed the SQLite per-connection pragma bug (R3) + the per-event all-workspaces webhook scan (H1). Remaining perf: H6 model cache, H4 outbox JOIN, R2 rate-limiter leak, H2 cursor coalesce; CI speedups (arm64 runner, build-once image, gha cache, trivy) once GitHub CI is back.

**Integrators:** use [Integration.md](Integration.md) — not this roadmap.

**Recently closed:** Cluster **166.0** — perf/correctness: per-connection SQLite pragmas (R3) + per-workspace webhook fan-out (H1); arc 2 part 1, at **`v166.0.0`**
([[Retros/Cluster 166.0]]).

**Recently closed:** Cluster **165.0** — reference authorization (REST + MCP `add_reference` gated on entity→channel access); RBAC arc complete, at **`v165.0.0`**
([[Retros/Cluster 165.0]]).

**Recently closed:** Cluster **164.0** — `channel:admin` capability + `/channels/:cid/members` membership API (REST+MCP); RBAC part F, at **`v164.0.0`**
([[Retros/Cluster 164.0]]).

**Recently closed:** Cluster **163.0** — verified WS/MCP subscribe grants (drop asserted private grants for non-members); RBAC part E, at **`v163.0.0`**
([[Retros/Cluster 163.0]]).

**Recently closed:** Cluster **162.0** — MCP aggregate-read filtering (search / list-channels / workspace-context); RBAC part D, at **`v162.0.0`**
([[Retros/Cluster 162.0]]).

**Recently closed:** Cluster **161.0** — private-channel access control over MCP (pre-dispatch gate on point-access content tools + `resources/read`); RBAC part C, at **`v161.0.0`**
([[Retros/Cluster 161.0]]).

**Recently closed:** Cluster **160.0** — private-channel access control over REST (`ensure_channel_access` on all content routes + search + workspace-context; creator auto-added); RBAC part B, at **`v160.0.0`**
([[Retros/Cluster 160.0]]).

**Recently closed:** Cluster **159.0** — channel membership model (`channel_members` table + store + migration, both backends; no enforcement); RBAC part A, at **`v159.0.0`**
([[Retros/Cluster 159.0]]).

**Recently closed:** Cluster **158.0** — keyless cosign signatures on the container images (server + postgres, by digest); enterprise-hardening arc part 3, at **`v158.0.0`**
([[Retros/Cluster 158.0]]).

**Recently closed:** Cluster **157.0** — fail-closed `AUTH_DISABLED` (explicit `MAIDAN_ALLOW_INSECURE_NO_AUTH` ack; never in prod); enterprise-hardening arc part 2, at **`v157.0.0`**
([[Retros/Cluster 157.0]]).

**Recently closed:** Cluster **156.0** — production-safety defaults (SIGTERM graceful shutdown + default 30 s `statement_timeout`); enterprise-hardening arc part 1, at **`v156.0.0`**
([[Retros/Cluster 156.0]]).

**Recently closed:** Cluster **155.0** — sampling-backed `summarize_thread` (first `request_client` caller; session id threaded through tool dispatch); closes lane 3 + the three-lane plan, at **`v155.0.0`**
([[Retros/Cluster 155.0]]).

**Recently closed:** Cluster **154.0** — `request_client` GET-stream delivery fix (per-session broadcast; server→client requests reach the canonical `GET /mcp/streamable`); lane 3 part 1, at **`v154.0.0`**
([[Retros/Cluster 154.0]]).

**Recently closed:** Cluster **153.0** — live-updating `/ui` thread view (WS message/reaction/pin frames → debounced `loadMessages`); UI polish lane, at **`v153.0.0`**
([[Retros/Cluster 153.0]]).

**Recently closed:** Cluster **152.0** — lean HTTP context pack (`MessageEditView`, opt-in `include_edits`) + `snippet_only` search; token-efficiency part 2 (REST parity), at **`v152.0.0`**
([[Retros/Cluster 152.0]]).

**Recently closed:** Cluster **151.0** — token-efficient lean context reads (`get_thread_context` edits metadata-only by default, opt-in `include_edits`; `list_messages` clamped `1..=500`), at **`v151.0.0`**
([[Retros/Cluster 151.0]]).

**Recently closed:** Cluster **150.0** — thread/member/kind filters on `GET /mcp/stream` (await my mention); completes the MCP-agent-surface pair, at **`v150.0.0`**
([[Retros/Cluster 150.0]]).

**Recently closed:** Cluster **149.0** — MCP inbox + mention tools (`list_mentions`/`get_inbox`/`mark_inbox_read`); an MCP-only agent can now discover its @mentions, at **`v149.0.0`**
([[Retros/Cluster 149.0]]).

**Recently closed:** Cluster **148.0** — MCP server→client requests (sampling/roots/elicitation, capability-gated) + client-capability tracking; **concludes the MCP streamable spec-completeness arc (145–148)**, at **`v148.0.0`**
([[Retros/Cluster 148.0]]).

**Recently closed:** Cluster **147.0** — MCP streamable resumability (SSE event ids + `Last-Event-ID` replay; session survives a dropped POST leg); part 3 of the MCP spec-completeness arc, at **`v147.0.0`**
([[Retros/Cluster 147.0]]).

**Recently closed:** Cluster **146.0** — `GET /mcp/streamable` server→client SSE + `Accept` content negotiation; part 2 of the MCP spec-completeness arc, at **`v146.0.0`**
([[Retros/Cluster 146.0]]).

**Recently closed:** Cluster **145.0** — MCP conformance basics (`initialize` version negotiation, `MCP-Protocol-Version` header, JSON-RPC batching + notifications); first of the MCP spec-completeness arc 145–148, at **`v145.0.0`**
([[Retros/Cluster 145.0]]).

**Recently closed:** Cluster **144.0** — docs dead-link gate (`mdbook-linkcheck` fails the build on broken internal links) + fixed 35 latent broken published links + backlog reconciliation, at **`v144.0.0`**
([[Retros/Cluster 144.0]]).

**Recently closed:** Cluster **143.0** — richer message rendering in the `/ui` thread view (timestamps + inline slash-command results) at **`v143.0.0`**
([[Retros/Cluster 143.0]]).

**Recently closed:** Cluster **142.0** — slash-command registry in the `/ui` console (register/list/revoke over `/ui/api`, new "Slash" tab; one-time secret for `http` handlers) at **`v142.0.0`**
([[Retros/Cluster 142.0]]).

**Recently closed:** Cluster **141.0** — fixed the published mdBook site (every `docs/*` sidebar link 404'd; now all 21 SUMMARY pages build + serve, via a build-time staging step) at **`v141.0.0`**
([[Retros/Cluster 141.0]]).

**Recently closed:** Cluster **140.0** — workspace presence roster in the `/ui` console (new "Presence" tab rendering the WS `presence_snapshot` frames; online/away controls) at **`v140.0.0`**
([[Retros/Cluster 140.0]]).

**Recently closed:** Cluster **139.0** — 1:1 direct messages in the `/ui` console (open/list/read/post over `/ui/api`, new "DMs" tab; parallel to group DMs) at **`v139.0.0`**
([[Retros/Cluster 139.0]]).

**Recently closed:** Cluster **138.0** — global-audit + reindex controls in the `/ui` "Operator" tab (bearer-gated audit; workspace/global reindex + poll), completing the operator-console arc, at **`v138.0.0`**
([[Retros/Cluster 138.0]]).

**Recently closed:** Cluster **137.0** — deliveries & DLQ operator view in the `/ui` console (list + status/kind filter + replay over `/ui/api`, new "Operator" tab) at **`v137.0.0`**
([[Retros/Cluster 137.0]]).

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
