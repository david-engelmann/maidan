# Path to impressive

**Pickup:** [Handoff.md](Handoff.md) to execute; this file is the strategy, not the checklist.

Strategic companion to Pre-Public Hardening. That doc is reputation polish.
This doc is product ambition: how Maidan becomes the tool people reach for
when agents need a shared workplace, without becoming a mediocre Slack clone.

Snapshot date: 2026-08-25. Program D closed at v266 (2026-08-24). The optional-deferrals sweep (**267–272**) has since shipped in full (tags through `v273.0.0`); the current backlog is [Open Work.md](Open%20Work.md). Expansion bets in [Expansion Bets.md](Expansion%20Bets.md).

---

## North star (decide this once)

Maidan wins as the best multi-agent collaboration substrate (durable shared
state, capability-scoped tools, task orchestration, HITL), not by matching
Slack chrome.

| Bet | Implication |
|-----|-------------|
| Agents primary; humans supervise | MCP/REST/A2A + evidence over SPA polish |
| Humans primary; agents are bots | Real UI, browser tests, mobile, Slack-parity UX |
| Bridge world | Humans in Slack/Teams; agents in Maidan |

Recommended: agents-first + thin HITL console + bridges. `/ui` is operator
surface, not the product. If you flip that, browser automation becomes P0.

---

## 1. UI testing and browser assurance

### What you have

- Static shell markers: ui_static_e2e, ui_search_e2e, ui_tokens_e2e, ui_channels_e2e
- Session `/ui/api` e2e (~1.6k lines across ui_*.rs): channels, threads, messages, admin, collab, edits
- WS via session: ui_ws_tail_e2e, parts of ui_v2_e2e
- ui_js_contract.rs: static analysis that bare JS calls resolve (Cluster 133 class). No browser.
- openapi_e2e lists `/ui/api` paths
- **Ten** `ui_*` test files. None of them is Playwright.

Implementation: one vanilla static/index.html (~2.4k lines). No SPA framework.

### What you do not have

- No headless browser job in CI
- No real DOM click-path coverage
- No screenshot / a11y / visual diff gates
- No proof client-side WS handlers work under a real browser event loop

### What to do

If `/ui` stays operator-thin: keep API + JS contract; optionally add one
headless smoke later (load UI, post message, assert row). Document that it is
an operator console. **UI browser e2e is still not the product** this quarter.

If humans live in `/ui`: budget months for real browser e2e — string checks
are not that.

---

## 2. Ecosystem gaps that limit adoption

### Already good

| Component | Today |
|-----------|-------|
| DB | Postgres + pgvector / SQLite |
| Artifacts | LocalFs + S3-compatible |
| Embeddings | hash-v1 + openai-compatible (OpenAI, Azure, Ollama, vLLM, TEI) |
| Auth | Capability bearers + OIDC (`token:admin` exists; keep it off agent tokens) |
| Transports | REST, MCP (**today** `2024-11-05`; **required** `2026-07-28` = J3), WS, A2A. [Protocols.md](Protocols.md) |
| Deploy | compose, Helm, binary, Pi/ARM64 |

### High bounce for target users

| Gap | Severity |
|-----|----------|
| No TS/Python client SDK | High — Expansion Bet 3 |
| No drop-in Cursor / Claude Desktop / VS Code MCP configs | High — **J3 then** Bet 2. Server is 2024-only **today**; that is the blocker, not missing JSON snippets. |
| No LangGraph / CrewAI recipes | High — recipes on REST+WS, not an in-process `Crew.kickoff` |
| No Slack/Teams bridge | High for distribution — Bet 1 projector |
| SAML/SCIM out of scope | Medium (document OIDC requirement) |
| SMTP email best-effort only (wired, no retry) | Medium — Bet 4; do not confuse with `maidan_outbox` |
| hash-v1 default can fool naive prod | Medium |
| MCP 2024-11-05 vs IDE 2026-07-28 | **P0.** Required upgrade J3. Do not launch on 2024. |

### Do not chase

External vector DBs as primary (Pinecone/Qdrant) — keeping vectors beside
RBAC messages is a feature. Native GCS/Azure blob only if demanded.
Search replica routing (271–272) is the other agent's job (`PostgresSearch`
has its own `PgPool`); not an expansion bet.

### Provider matrix (the "whatever they already run" job)

Users will not adopt a workspace that forces one cloud's Postgres and
OpenAI. They also will not wait for a third database engine.

**Dialects we keep (two, not N):** Postgres and SQLite. Production HA,
LISTEN bus, `pgvector`, LSN replicas = Postgres. Laptop / Pi / tests =
SQLite. A MySQL/Mongo backend would duplicate the 228-method `Store`
and still lack NOTIFY/pgvector. That is a no.

**Hosts we owe a tested page for** (same code, different URLs):

| Surface | Already in code | Prove these hosts |
|---------|-----------------|-------------------|
| DB | `DATABASE_URL` Postgres or SQLite | RDS, Aurora, Neon, Supabase, Cloud SQL, Crunchy — plus "enable pgvector." SQLite file / memory / Pi. |
| Embeddings | `hash-v1` + `openai-compatible` | OpenAI, Azure OpenAI, Ollama, vLLM, TEI. One protocol. Chat models stay in the *agent*, not Maidan. |
| Artifacts | LocalFs + S3-compatible | MinIO, AWS S3, Cloudflare R2. Native GCS/Azure only if S3 interop fails. |
| Auth | Generic OIDC | Keycloak + one SaaS IdP. SAML out. |
| Mail | SMTP | Any relay (SES/SendGrid as SMTP). Native HTTP APIs later/never. |

Execution checklist: Hardening **I** (`docs/Providers.md`, then recipes).
Do not add embedding SDKs or a third `Store` impl.

### Protocol matrix (the "whatever they already speak" job)

Users will not adopt a workspace that invents a fourth agent protocol.
They also will not wait for GraphQL, gRPC-for-REST, or IBM ACP.

**Layers we keep (industry 2026, AAIF):** MCP = tools, A2A = peers,
REST/OpenAPI + WS + webhooks = existing IT. AG-UI is a frontend protocol
we do not native-ize until `/ui` is the product. Zed ACP is an optional
*worker* adapter (OpenTag-shaped), not the workspace.

**Wires we owe honesty on** (same code, clearer contract):

| Surface | Already in code | Prove / fix |
|---------|-----------------|-------------|
| MCP | `2024-11-05` JSON-RPC + session Streamable HTTP + stdio | Freeze copy (J2 / Bet 2 M.0). Dual-negotiate `2026-07-28` only when sessions are honest (J3). |
| A2A | JSON-RPC v1.0 subset, custom Agent Card, text-only egress | Card `supportedInterfaces` (J4). File parts (J5). No gRPC unless a cloud blocks. |
| REST + WS + webhooks | OpenAPI, subscribe, signed POSTs | n8n recipe (J7). LangGraph/CrewAI recipe on these, not in-process (J8). |
| Slack Events | none | Bet 1 projector, not a protocol rewrite. |
| GitHub App / GitLab webhook | none | Bet 6 projector. Official GitHub MCP stays the repo tool. |
| ACP / AG-UI / ANP / AP2 | none | Adapter or watch. Do not native. |

Execution checklist: Hardening **J** (`docs/Protocols.md`). Do not add a
Maidan-native agent protocol.

### Do

Publish both matrices (hosts + wires). Ship interop packs (MCP JSON, thin SDK, framework
recipes) **after** documenting the MCP protocol freeze.

---

## 3. Slack gaps (triage)

Skip: huddles/voice, native mobile, emoji packs, Workflow Builder UI,
org hierarchy, SAML-in-Maidan, SCIM.

Worth for HITL: notification polish, rich unfurls/task cards for supervisors.

Already ahead of Slack (market these): DAG, leases, skill claim,
`claim_next_thread`, `wait_for_*`, structured results, tool transcripts,
capability tokens, context export, A2A, at-least-once, read-your-writes tokens.

---

## 4. Steal from other platforms

| Source | Idea | Fit |
|--------|------|-----|
| Linear | Issue-shaped threads | Native strength |
| GitHub / GitLab | Issue/PR mention → thread → comment/Check Run | **Bet 6 projector**, not Copilot |
| Discord | Role/skill aliases on ACL | Partial |
| LangGraph | Shared durable checkpoint across workers | Interop essay + SDK wrapping REST+WS |
| PagerDuty/Sentry | Alert to thread to `claim_next_thread` | Recipe |

Highest-ROI **for us** (Claude agent owns import + search routing): Slack
projector, client SDKs + MCP pack (protocol honesty then `examples/`),
hero multi-agent demo (offline DAG, no LLM), durable mail retry.
Workspace import is 269–270 — not our bet. `/ui` SPA is not the product.

---

## 5. Performance, load, and optimization

This is a **code-improvement** track (Hardening **H**), not an expansion
bet and not more Program D.

**Already shipped:** sharded fan-out, filtered ANN, concurrent context,
transactional **bus** outbox (`maidan_outbox`, 205/84), NOTIFY self-heal,
LSN read replicas (**265–266 closed** at v266), `scripts/loadgen.sh` +
`#[ignore]` `load_baseline` (198), criterion `search_hot` / `store_hot`
(109 / 120), scale-out smoke + `replica-harness.sh`.

**Do not "finish 266."** It shipped. Search token-aware replica routing
is 271–272 (other agent; `PostgresSearch` owns its pool).

**The remaining job** is in [Pre-Public Hardening.md](Pre-Public%20Hardening.md)
section **H**:

1. Record a Postgres loadgen baseline (today the default is SQLite
   in-process, REST post/read/search only).
2. Extend the mix to MCP / WS / `claim_next_thread` (the agent path).
3. Optimize **only** what those numbers move: context filter-before-build,
   cheaper search deny-set, maybe DM-in-SQL. Mail retry is Bet 4, not perf.
4. Fix Production.md, which still says load/throughput is "not covered."

Do not SPA-rewrite `/ui` for speed. Do not add Redis until Evidence shows
a bus/DB bottleneck. Do not chase external vector DBs for vanity benches.
Do not reopen batched `pg_notify` (declined in Open Work).

---

## 6. Most useful tool possible

useful ≈ agent outcomes × ease of adopt × trust

| Factor | Levers |
|--------|--------|
| Outcomes | DAG, skills, waits, context, A2A, leases — keep deepening |
| Adopt | examples/, SDKs, MCP packs (2024-11-05 honesty first), bridges, secure quickstart, provider matrix, protocol matrix |
| Trust | Pre-Public Hardening, Evidence/Gates, threat model |

### 90-day sequencing

The Claude agent shipped **269** and is finishing **270–272** (import REST+remap+409 /
search token-aware replica routing). That is **not** our quarter. Do not
duplicate it. Hardening P0 can overlap.

**Our quarter is pack-and-prove, then Slack projector.** `/ui` browser e2e
is still not the product.

- **Now (parallel with 270–272):** Hardening P0 (tone, README first
  command, `mail.rs` lie) **and H1/H5** (Postgres loadgen baseline +
  Production.md honesty) **and I1/J1** (`docs/Providers.md`,
  `docs/Protocols.md`). Measurement and docs do not collide with import PRs.
- **Then MCP 2026, then pack:** Hardening **J3** / Bet 2 **M.0** is the
  required `2026-07-28` upgrade (not a 2024 freeze). **M.1** `examples/` +
  2026 snippets. **M.2**
  offline DAG seed using existing hero tools (`claim_next_thread`,
  `wait_for_result`, `post_message`, `set_thread_result`,
  `request_approval`, `search_messages`) — no new runtime, no LLM.
  Optional Bet 3 TS client, pinned to a 7-method OpenAPI freeze, ≤15
  methods, REST+WS, not `Crew.kickoff`.
- **Then (if claiming the front door):** Bet 1 Slack projector
  (HTTP Events, mention-only, final-message first). Bet 4 mail retry
  only if we claim reliable notifications (new `mail_outbox`).
- Star-tax (GIF / topics / homepage) stays parked.

### Impressive to each audience

| Audience | They should say |
|----------|-----------------|
| Agent engineer | Connected in five minutes; agents share work for real |
| Staff eng | Dual-backend, capability CI, delivery semantics — serious |
| Ops | Helm, probes, replicas, backups, signed releases |
| HN skeptic | Not a Slack clone — infrastructure for agent teams |

---

## Decision checklist

1. Primary user: agent or human? (Recommend agent.)
2. Humans in Slack via bridge, or in `/ui`? (Recommend bridge.)
3. Next quarter: package-and-prove after D — **D is done**; optional-deferrals (269–272) are the other agent's in-flight work; **then package** (examples + MCP honesty + optional SDK). Not more substrate. Not `/ui` SPA.
4. First interop bet: MCP pack, SDK, or Slack bridge? (Recommend pack M.0→M.2, then SDK, then bridge.)

Record answers in Decisions.md when picked.

---

## See also

- [Pre-Public Hardening.md](Pre-Public%20Hardening.md)
- [Expansion Bets.md](Expansion%20Bets.md) — researched Slack/MCP/SDK/mail bets after 270-272
- [Protocols.md](Protocols.md) — 2026 integration wires vs what we speak
- [Providers.md](Providers.md) — host matrix
- [Launch.md](Launch.md) — public cut, production-ready extras, announce
- [Handoff.md](Handoff.md) — pickup page for a later agent session
- [Open Work.md](Open%20Work.md) / [Remaining Work.md](Remaining%20Work.md)
- [Embeddings.md](Embeddings.md) / [Production.md](Production.md) / [Threat-Model.md](Threat-Model.md)
- [AGENTS.md](../AGENTS.md)
