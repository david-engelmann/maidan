# Expansion bets (researched)

Companion to `docs/Pre-Public Hardening.md` (polish) and `docs/Path to Impressive.md`
(strategy). This file is the **feature-expansion** backlog: each bet is a new
flow or product surface, documented with 2026 market evidence, a Maidan-shaped
design, MVP vs later, risks, and cluster-sized slices.

**Written:** 2026-08-25. **Code baseline:** **267–271 shipped** (`v271.0.0` = `main`).
**272** is [#522](https://github.com/david-engelmann/maidan/pull/522), code committed, waiting CI.
Program D closed at **v266**. **Tree audit:** 2026-08-25 local `rg`/`wc`
(13 crates; `Store` 228 methods; MCP 78 tools, **2024-11-05-only today — J3 required**; residue 771).

**Pickup:** [Handoff.md](Handoff.md) is the session start page (master IDs + try-out matrix). Public cut / announce: [Launch.md](Launch.md).

**How to use:** do **not** start an expansion bet while the optional-deferrals
sweep (269–272) is still the other agent's active ladder. Hardening P0 (tone,
README first command, stale `mail.rs` module docs) **can** run in parallel.
When that sweep ends, pick **one** expansion bet. Star-work stays parked
unless you reopen it. This roadmap stays **outside** 269–272.

---

## 0. What changed since the last research round (v266)

| Cluster | Status | What it was in the last rec set |
|---------|--------|----------------------------------|
| **267** A2A egress `content → parts` | **Shipped** `v267.0.0` | Listed as leftover “federation egress” |
| **268** MCP email-address tools | **Shipped** `v268.0.0` | Listed as low-value Arc I leftover |
| **269** workspace import **store** | **Shipped** `v269.0.0` | Listed as portability hole |
| **270** import REST + remap + 409 | **shipped v270** | Completes 269 |
| **271** search token-aware replica routing | **shipped v271** | `PostgresSearch` own pool |
| **272** search replica-reads counter | **#522 waiting CI** | Do not duplicate; do not add this pack onto that PR |

**Rescore of last round**

| Prior rec | Now |
|-----------|-----|
| Finish Program D | **Done** (v266) |
| Close A2A egress | **Done** (v267) |
| MCP email tools | **Done** (v268) — they took the parity even though we called it low-value |
| Workspace import | **269 store shipped**; **270 HTTP in flight** — do not duplicate |
| Search replica routing | **271 shipped; 272 = #522 waiting CI** — do not duplicate |
| Durable email retry queue | **Still open** (249 was best-effort, no retry) |
| Slack teammate / Claude Tag front door | **Still the highest-leverage expansion** |
| Cursor/Claude MCP pack + hero demo | **Still the cheapest adoption expansion** |
| TS/Python client SDK | **Still open** |
| `/ui` browser e2e / SPA | **Still not the product** unless humans live in `/ui` |
| GitHub topics/GIF/homepage | **Star-tax**, parked per 2026-08-24 decision |
| Pre-public hardening (residue, module splits) | **Still valid after the sweep** — not an expansion |

The other agent is executing a clean **optional-deferrals sweep**. Let them finish
270–272. Expansion work starts after. Polish P0 can overlap.

### Market snapshot (GitHub API, 25 Aug 2026)

| Product | What it is | Stars 25 Aug 2026 |
|---------|------------|-------------------|
| Claude Tag | Closed Slack-hosted shared @Claude | n/a |
| [amplifthq/opentag](https://github.com/amplifthq/opentag) | Local ACP dispatcher (Slack mention to Claude Code/Cursor on your machine) | 1,367 |
| [korotovsky/slack-mcp-server](https://github.com/korotovsky/slack-mcp-server) | Slack-as-MCP (OAuth or stealth xoxc/xoxd) | 1,794 |
| [paradigmxyz/centaur](https://github.com/paradigmxyz/centaur) | OSS Claude Tag-style agentic infra | 1,182 |
| [fancyboi999/open-tag](https://github.com/fancyboi999/open-tag) | Self-hosted Slack-style workspace (closest public cousin) | 170 |
| [openma-ai/open-managed-agents](https://github.com/openma-ai/open-managed-agents) | Self-hosted Tag-style runtime | 248 |
| [agentconnect-md/agentconnect](https://github.com/agentconnect-md/agentconnect) | Multi-agent Tag alt; ACP; Slack+GitHub | 152 |
| [Anil-matcha/open-claude-tag](https://github.com/Anil-matcha/open-claude-tag) | Tiny Tag clone (created 17 Aug 2026) | 4 |
| [MeetQuinn/anima](https://github.com/MeetQuinn/anima) | Local teammate runtime, own Slack identity | 3 |
| [ACP spec repo](https://github.com/agentclientprotocol/agent-client-protocol) | Editor to agent JSON-RPC (Rust, v2 draft 20 Jul 2026) | 4,068 |
| [a2aproject/A2A](https://github.com/a2aproject/A2A) | Agent to agent | 25,488 |
| [CrewAI](https://github.com/crewAIInc/crewAI) | Role-based Python multi-agent DX | 57,598 |

Naming collisions: amplifthq/opentag (the 1.3k one) vs CopilotKit/OpenTag vs fancyboi999/open-tag vs Anil-matcha/open-claude-tag. Do not confuse them.

**Positioning:** Claude Tag is a cloud Claude that lives in Slack. OpenTag is a local ACP dispatcher that lives in Slack threads. Maidan is the self-hosted workspace those agents should work IN, with a Slack projector, a one-click MCP pack, and a Crew-shaped SDK, none of which are the product.

---

## Codebase constraints (audited 2026-08-25)

This section is a **local tree audit**, not a wish list. Bets below must reuse
what exists, name symbols correctly, and stay off the other agent's ladder.

**Claude agent owns (do not duplicate):** 269–271 **shipped**. **272** is #522 (search replica-reads metric), waiting CI. Stay off `maidan-search` / `metrics.rs` / `state.rs` until it merges. J3 does not collide (no `maidan-mcp` in 272).

**We own (this roadmap):** polish (Hardening) + expansion bets 1–5 below.

### What exists

**13 crates:** `maidan-a2a`, `maidan-artifacts`, `maidan-auth`, `maidan-bus`,
`maidan-cli`, `maidan-fsm`, `maidan-mcp`, `maidan-observability`,
`maidan-router`, `maidan-search`, `maidan-server`, `maidan-store`,
`maidan-types`. No `slack` / `bridge` / `acp` crate (docs only). No
`examples/` tree. No in-repo `mcp.json` snippets.

**Naming (wire / MCP / REST / store trait — use these, not `claim_next`):**

| Surface | Symbol |
|---------|--------|
| MCP tool + REST | `claim_next_thread` (`crates/maidan-mcp/src/tools/catalog.rs`, `crates/maidan-server/src/routes/thread.rs`) |
| Store trait | `claim_next_thread` / `claim_next_thread_with_event` (`crates/maidan-store/src/store.rs`) |
| Internal SQL helper | `threads::claim_next` (Postgres/SQLite) — not the public name |
| EventKind | `MessagePosted`, `ThreadResultSet`, `MentionRecorded` (`crates/maidan-types/src/events.rs`) |
| Wire event names | `message_posted`, `thread_result_set`, `mention_recorded` |

**MCP today** is a **2024-11-05-only** server. That is a **P0 upgrade**,
not a freeze we ship:

- `SUPPORTED_PROTOCOL_VERSIONS = ["2024-11-05"]` in
  `crates/maidan-mcp/src/server.rs` (~line 30).
  `negotiate_protocol_version` will not accept `2026-07-28`. **J3 / M.0
  must change this.**
- `GET /mcp/streamable` + `Mcp-Session-Id` are still first-class
  (`crates/maidan-server/src/mcp_streamable.rs`,
  `crates/maidan-mcp/src/streamable_session.rs`). Spec **2026-07-28
  removed GET stream + protocol-level sessions**. The upgrade has to
  make Streamable HTTP honest, not sticker 2026 on the old session model.
- `catalog.rs` (~989 lines) lists **78 tools**. It is **not** a REST
  projection: there is no MCP create workspace / channel / thread /
  member, and no group DM. An MCP-only agent **cannot bootstrap** a
  workspace. Hero demo seed must use REST / `maidan` CLI / compose,
  then MCP for claim/wait/post. Do not add create-* MCP tools just
  for the demo unless that is an explicit extra slice.
- Kitchen sink already includes `claim_next_thread`, `wait_for_result`,
  `wait_for_ready`, `wait_for_mention`, `wait_for_notification`,
  `set_member_email` / `get_member_email` / `delete_member_email`,
  `request_approval`, `search_messages`, `post_message`,
  `set_thread_result`. A hero pack **subsets** this catalog. It does
  not add tools.
- There is **no shared schema** across utoipa OpenAPI, hand-rolled
  `catalog.rs` JSON, and a future SDK. Freeze one source of truth
  before generating clients (Bet 3 C.1).

**Outbox vs mail — do not conflate:**

- `maidan_outbox` + `crates/maidan-server/src/outbox_relay.rs` is the
  **event-bus transactional outbox** (Clusters 205 / 84). Schema is bus
  events (`log_id` → `maidan_events`). Attempts + quarantine exist.
  It is **not** a generic job queue.
- Mail is `crates/maidan-server/src/mail.rs` (lettre SMTP), config-gated
  on `MAIDAN_SMTP_HOST` + `MAIDAN_SMTP_FROM`.
  `notification_router.rs` `tokio::spawn`s `deliver_notification_email`;
  comments say best-effort, never retried, durable queue is a follow-up.
- Digest mode (255) and presence skip (253) **already exist**. Bet 4 is
  retry / DLQ, not inventing email.

**Search:** `PostgresSearch { pool: PgPool }` owns its pool
(`crates/maidan-search/src/postgres.rs`). It is **not**
`store.read_pool`. Token-aware replica routing is 271–272 — the other
agent's job. Do **not** add it as an expansion bet.

**A2A egress (267):** `content → parts` is **text-only** today
(ingress `parts → content` was 194). Do not design Slack/SDK as if
file/data parts already round-trip on A2A.

**Auth:** `token:admin` exists
(`crates/maidan-auth/src/capability.rs` `TOKEN_ADMIN`). Slack-bridged
agents and SDKs must never mint it.

**OpenAPI:** utoipa; `crates/maidan-server/src/openapi/paths/api.rs` is
844 lines. An SDK is feasible from this, but freeze a **7-method
subset** rather than generating the whole kitchen sink.

**README / tone:** first command is
`DATABASE_URL=sqlite::memory: cargo run --bin maidan-server`.
`docker compose --profile full` is later. `CONTRIBUTING.md` and
`SECURITY.md` still open with "Maidan is pre-release". Coverage floor
`COVERAGE_MIN_LINES=40`. Ten `ui_*` test files; `ui_js_contract.rs` is
static analysis, no Playwright.

**Store tax:** the `Store` trait is **228 methods** (`store.rs` 1057
lines). Any new Slack/mail table is trait + postgres + sqlite + dual
migration + parity. Slack MVP should keep bindings in a server module
(or a small crate) and **not** add 20 Store methods on day one.

**Version story:** workspace `Cargo.toml` is `version = "0.0.0"` and
`publish = false`. Product versions are git tags (`v269.0.0`). There
is no crates.io/SDK depend story. Hardening P1.

**Reliability:** clippy `-D unwrap_used` on lib/bins. One production
`panic!` (`state.rs` subscribe-resume secret). Mail is fire-and-forget.
Eight copy-pasted worker loops; none is a generic job runner.

**Residue:** **771** `Cluster ` comments in `crates/` (301 server, 294
store; was ~754 on 8/24). `models.rs` has 33 Cluster refs. Almost no
`TODO` / `FIXME` / `todo!()` in production Rust — unfinished feel is
narrative, not stubs.

**Wikilinks:** still in `Integration.md`, `Production.md`, `AGENTS.md`,
and (until this edit) Pre-Public Hardening itself (`[[Open Work]]`).
GitHub renders them as dead text. This file stays GitHub markdown.

### What is a lie / stale comment

`mail.rs` module docs still say **"Not wired into the notification
router yet"**. That is false as of Cluster 249. The router *is* wired;
the remaining hole is retry. Fix the comment in Hardening P0 (Bet 4 E.1
also owns it if you touch mail). Do not design Bet 4 as if email
delivery does not exist.

### What this forces on each bet

| Bet | Constraint from the tree |
|-----|--------------------------|
| **1 Slack** | Projector over existing `claim_next_thread` / `wait_for_mention` / `MessagePosted` / `ThreadResultSet` / `MentionRecorded`. No LLM in Maidan. No new agent runtime. Capability-scoped bot token, never `token:admin`. |
| **2 MCP pack** | Protocol honesty first (M.0). Hero tools already exist — subset, don't add. Seed the demo workspace via REST/CLI (MCP cannot create workspace/channel/thread). `examples/` does not exist; that *is* the pack work. No 2026-07-28 deeplinks until the server negotiates that rev **and** GET-session semantics are decided. |
| **3 SDK** | Wrap REST + WS. Map waits to `claim_next_thread` / `wait_for_result`. Do not invent `Crew.kickoff` as an in-process runtime. Pin to a protocol / OpenAPI freeze. 15 methods max, TS first. Import is method 8 *after* 270. |
| **4 Mail retry** | Do **not** "reuse the outbox worker" as if it were generic. New `mail_outbox` (or a job kind that is **not** `maidan_outbox` events). Presence / digest already handled. |
| **5 Pack-and-prove** | Polish, not features. Monster-file splits and residue 771 live in [Pre-Public Hardening.md](Pre-Public%20Hardening.md). |

---

## Bet 1 — Slack teammate (Claude Tag-shaped front door)

**Priority:** highest *category / star ceiling*. **Not first to build** — pack (Bet 2) then SDK (Bet 3) then this projector. See sequence below.
**Do not start until:** 270–272 land (or you explicitly pause that sweep).

### Problem / who cares

Humans will not install Maidan as their chat app. They already live in Slack
(or Teams). The 2026 winning pattern is: **one shared agent identity in the
channel the humans already have**, not a new workspace they must open.

Maidan already is the *serious* multi-agent workplace (DAG, leases, capabilities,
durable memory, MCP). It is missing the *front door* those humans walk through.

### 2026 market evidence

**Claude Tag (Anthropic + Slack), 2026-06-23.**
Source: [Introducing Claude Tag](https://www.anthropic.com/news/introducing-claude-tag) (fetched 2026-08-25).
Beta for Claude Enterprise/Team on Opus 4.8. One shared `@Claude` per channel (multiplayer, pick up mid-task); channel-scoped identities so sales vs eng do not share memory or tools; admin spend limits (org + per-channel); optional ambient/proactive follow-ups; async tasks over hours/days; DMs with personal tools. Replaces the old Claude-in-Slack app (30-day migrate). Anthropic claims 65% of their product team code is created by internal Claude Tag. Category-defining, closed, vendor-locked.

Coverage: [VentureBeat](https://venturebeat.com/technology/anthropic-launches-claude-tag-replacing-its-slack-app-with-a-persistent-ai-teammate-that-learns-monitors-and-works-autonomously),
[TechRepublic](https://www.techrepublic.com/article/news-anthropic-claude-tag-ai-agent-slack/).

**Open-source clones riding that wave (star counts move; treat as order-of-magnitude):**

| Project | Shape | Why it stars |
|---------|--------|--------------|
| [amplifthq/opentag](https://github.com/amplifthq/opentag) (~1.3k stars, created 2026-06-24) | Mention in Slack/GitHub → run Claude Code/Codex/Cursor via **ACP** on your machine → reply in-thread | “Your agent, your laptop” |
| [Anil-matcha/open-claude-tag](https://github.com/Anil-matcha/open-claude-tag) | Self-host Slack teammate, MEMORY.md, LLM-agnostic | Explicit “OSS Claude Tag” |
| [korotovsky/slack-mcp-server](https://github.com/korotovsky/slack-mcp-server) (~1.8k) | MCP over Slack history/post | Cursor-today, not a teammate |

**ACP (Agent Client Protocol)** — [agentclientprotocol.com](https://agentclientprotocol.com/),
[github.com/agentclientprotocol/agent-client-protocol](https://github.com/agentclientprotocol/agent-client-protocol).
JSON-RPC (MCP-adjacent types) between editors and coding agents. Local stdio or
remote HTTP/WS. v1 stable, v2 draft 2026-07-20. OpenTag uses ACP to talk to
Cursor/Claude Code/Codex. **Maidan should not reimplement ACP as the workspace**;
it should optionally *dispatch* an ACP agent as a *worker* on a Maidan thread.

### Why Maidan is well-positioned

Claude Tag / OpenTag / open-claude-tag store “memory” as channel logs or
`MEMORY.md`. Maidan already has:

- Threads-as-tasks + FSM + `claim_next_thread` + leases (171, 190–192)
- DAG + `wait_for_ready` / `wait_for_result` (217–236)
- Skill routing (230–233)
- Per-recipient notifications (237–257)
- Capability-scoped tokens (so a Slack-bridged agent is not god-mode; never `token:admin`)
- HITL `request_approval` elicitation (174)
- Structured content + tool transcripts (173, 197)

The gap is **ingress from Slack + streamed egress to Slack**, not another
memory store.

### Disadvantage

You are not a Slack app today. OpenTag already has the mention UX. If the
bridge is slow, echo-loopy, or can’t stream, you lose the category even with
a better backend.

### Concrete design (Maidan-shaped)

**Identity mapping**

- One Slack workspace ↔ one Maidan workspace (install-time).
- One Slack channel ↔ one Maidan channel (lazy-create on first `@maidan`).
- Slack thread `ts` ↔ Maidan `Thread` (create on mention; store `slack_channel_id`
  + `slack_thread_ts` on the thread or a `maidan_bridge_bindings` table).
- Slack user ↔ Maidan `Member` `kind=human` (OIDC later; for MVP a hashed
  `slack_user_id` member with `slack:` handle).
- Bot user ↔ Maidan `Member` `kind=agent` with a **capability-scoped** token
  (`message:post`, `thread:transition`, `workspace:read` — never `token:admin`).

**Event path (ingress)**

1. Default **Events API HTTP** in production (Slack posts to `/bridges/slack/events`). Slack Marketplace **requires** HTTP Events; Socket Mode is **forbidden** for Marketplace and capped at 10 sockets/app. Socket Mode is laptop/airgap only. Source: [HTTP vs Socket Mode](https://docs.slack.dev/apis/events-api/comparing-http-socket-mode).

2. **Ack immediately** (HTTP 200 in <3s, ideally <200ms). Slack retries up to 3
   times on timeout ([Events API failure behavior](https://docs.slack.dev/apis/events-api.md)).
   Enqueue work; never run the agent in the request handler.
3. Idempotency key = Slack `event_id` (same on retries). Unique index on
   `maidan_bridge_inbox(event_id)`.
4. Filter `bot_message` / own `bot_id` or you echo-loop.
   ([RunGuard writeup](https://runguard.dev/blog/slack-ai-workflow-builder-cost-control.html)).
5. On `@maidan` / app_mention: `post_message_with_event` into the bound thread
   (or create thread), then `claim_next_thread` **or** emit `MentionRecorded`
   (`mention_recorded` on the wire) so an MCP agent already
   `wait_for_mention` / `wait_for_notification` wakes.

**Do not put an LLM in Maidan.** Maidan remains substrate. The “teammate”
is whichever agent is connected over MCP/A2A/ACP and claiming work via
`claim_next_thread`. That is the differentiator vs Claude Tag (locked to
Claude) and vs OpenTag (locked to a local coding agent). Still true after
the 2026-08-25 audit: no model host, no `Crew.kickoff` runtime.

**Egress (streamed reply)**

Slack shipped native streams on 7 Oct 2025: `chat.startStream` (Tier 2, 20+/min), `chat.appendStream` (higher tier; confirm), `chat.stopStream`. MUST be a threaded reply (`thread_ts`). Chunks include markdown_text, task_update, plan_update.
Do NOT `chat.update` a message that is currently streaming (`streaming_state_conflict`). Do NOT map Maidan MCP tokens 1:1 onto Slack; coalesce 200-500ms; map DAG node changes to task_update/plan_update; finalize with stopStream + Maidan permalink.
On 429, fall back to a single `chat.postMessage` of the final answer (OpenClaw pattern). Legacy `chat.update` is the fallback, not the design.
Sources: [chat streaming changelog](https://docs.slack.dev/changelog/2025/10/7/chat-streaming) and [chat.startStream](https://docs.slack.dev/reference/methods/chat.startstream).

Implementation: subscribe to the Maidan thread (`at_least_once` WS or MCP SSE). Buffer agent `MessagePosted` / `ThreadResultSet` (`message_posted` / `thread_result_set` on the wire). Coalesce 200-500ms into `appendStream` chunks. Last flush is `stopStream` plus a Maidan permalink (Block Kit task card is later HITL).

**HITL**

Slack block actions (`approve` / `reject`) → Maidan `request_approval` result or thread transition. Keep the durable decision in Maidan; Slack is the button surface.

**Ambient: default OFF.** Mention-only is MVP. Claude Tag ambient/proactive follow-ups are a reputation minefield for a small OSS project; do not ship them until mention/echo/retry tests are boringly green and an operator has opted in per channel.

**Auth**

- Slack OAuth v2 install (bot token + optional user token).
- Secrets in existing federation/keyring style (`FEDERATION_ENCRYPTION_KEY`
  / decrypt keyring from Cluster 189) — do not add a third crypto path.
- Per-channel allowlist (Claude Tag’s admin grant model).

### MVP vs later

**MVP (2-4 clusters)**

1. Binding table + Slack Events HTTP + ack/idempotency + bot-loop filter.
2. `app_mention` to Maidan thread + message; agent via existing MCP `wait_for_mention` still works (no new agent runtime).
3. S.4 projector: `chat.postMessage` of the agent final `ThreadResultSet` (final-message only).
4. Docs: Slack app manifest, scopes (`app_mentions:read`, `chat:write`, `channels:history`), compose profile. Marketplace path is HTTP Events only.

S.5 is native streams (`startStream` / `appendStream` / `stopStream`), not a later `chat.update` cadence. Use native streams as soon as the final-message projector is green, or keep S.4 as the first projector if streams are too much for the first slice. Legacy `chat.update` is fallback only (429 / `streaming_state_conflict`).

**Later**

- Block Kit task cards + approve/reject (S.6)
- GitHub issue comment ingress — **moved to Bet 6** (Git projector). Do not build it as a Slack leftover.
- ACP worker adapter (dispatch Cursor/Claude Code onto a claimed thread)
- Socket Mode for `maidan-cli slack-dev` (laptop/airgap only; never Marketplace)
- Channel follow to Slack channel mute/notify mapping
- Ambient/proactive follow-ups (opt-in, per-channel; default OFF)

### Risks / non-goals

- **Non-goal:** replacing Slack. Non-goal: huddles, emoji, Slack Connect UX.
- **Echo loops** and **retry duplication** are the production-killers; tests
  must cover them before any public claim.
- **Rate limits:** 30k Events deliveries / workspace / app / 60 min;
  writes ~1/s/channel. A busy `#eng` will need coalescing.
- **Compliance:** Slack ToS + storing Slack message bodies in Maidan — document
  retention (Cluster 186) applies; don’t silently keep forever.
- **Don’t** scrape Slack session tokens (“stealth mode”). That’s how some MCP
  Slack servers got stars; it will not reflect well on you as an engineer.

### Suggested slices

| Cluster | Scope |
|---------|--------|
| S.1 | `maidan_bridge_bindings` + `maidan_bridge_inbox` (event_id unique); no Slack I/O |
| S.2 | HTTP Events endpoint + signature verify + ack + enqueue |
| S.3 | Mention → thread/message (reuse `post_message_with_event`) |
| S.4 | Result → `chat.postMessage` (final only) + Production/Integration docs |
| S.5 | Native `chat.startStream` / `appendStream` / `stopStream` + Block Kit card |
| S.6 | Slack interactive HITL |

---

## Bet 2 — MCP pack + hero multi-agent demo

**Priority:** cheapest *adoption* expansion (Cursor / Claude Desktop / any MCP host).
**Stars it earns:** fewer than Slack, but this is how engineers *try* Maidan tonight.

### Problem

`AGENTS.md` to `docs/Integration.md` is the integrator path. It is still "read a
book, mint a token, wire MCP." Cursor and Claude Desktop want a **one-click
MCP server** plus a **30-second demo** that proves multi-agent is not a
slide.

The 2026-08-25 tree makes the gap concrete: **no `examples/`**, **no
`mcp.json` snippets**, and the server still speaks **only MCP 2024-11-05**
while current IDE clients may speak **2026-07-28**. Shipping a deeplink
that implies Streamable HTTP 2026-07-28 against this binary is a lie.

### 2026 market evidence

- MCP is the default plugin surface in Cursor, Claude Desktop, and a growing
  set of IDEs. Stars accrue to *servers people add in five minutes*
  (`slack-mcp-server` ~1.8k) more than to substrate they have to operate.
- Agent frameworks that *feel* like products (CrewAI ~50k, LangGraph ~20-30k,
  OpenHands ~80k+) ship a hero: "researcher to implementer to reviewer" or
  "issue to PR." Maidan has the primitives (DAG 217-236, `wait_for_result`,
  skills 230-233) but no canned story.
- A2A (Google/Linux Foundation) is the *agent-to-agent* protocol Maidan
  already speaks. MCP is the *host-to-Maidan* protocol. Do not confuse them
  with ACP (Bet 1). Three protocols, three jobs:
  - **MCP** — Cursor talks to Maidan
  - **A2A** — Maidan talks to a remote agent runtime
  - **ACP** — Maidan (optionally) dispatches a *local coding agent*

### Design (reuse, don't invent)

**M.0 — MCP `2026-07-28` (required; this is Hardening J3)**

**Staying on `2024-11-05` is not acceptable.** Modern Cursor/Claude
clients negotiate `2026-07-28`. M.0 is not "document 2024 and wait."
M.0 **is** the upgrade cluster (J3):

- `negotiate_protocol_version` / `initialize` current = `2026-07-28`
- Streamable HTTP: `Mcp-Method` + `Mcp-Name`; no protocol-level session
  required for 2026 clients
- GET `/mcp/streamable` + `Mcp-Session-Id` are **not** 2026 Streamable HTTP;
  live-wait stays `/mcp/stream` / WS / `wait_for_*`
- Then (only then) Cursor/Claude/VS Code deeplinks and `examples/` snippets

That is *not* a docs-only pack task. Do not sneak it into M.1. A 2024-only
pack is marketing on a mismatch. Optional: accept old 2024 initialize for
one release if it does not restore the session lie.

**Pack (M.1–M.2) — subset, do not add tools**

Hero tools **already exist** in `catalog.rs`. The pack is a documented
subset + snippets, not a new runtime:

- `claim_next_thread`
- `wait_for_result`
- `post_message`
- `set_thread_result`
- `request_approval`
- `search_messages`

Kitchen-sink tools (`wait_for_ready`, `wait_for_mention`,
`wait_for_notification`, `set_member_email` / `get_member_email` /
`delete_member_email`, …) stay on the server. The hero path does not
advertise them.

There is **no `examples/` today**. That is the actual pack work:

1. `examples/cursor-mcp.json` + `examples/claude-desktop.json` (and a VS
   Code `.vscode/mcp.json` fragment if you ship that surface) pinned to
   the frozen protocol rev.
2. Seed script uses existing store APIs (`create_workspace`,
   `create_channel`, `create_thread`, `post_message_with_event`, DAG
   edges) and the hero tools above. **No new runtime. No LLM.**
3. README *above* `cargo run`: MCP snippet, with docker-or-binary as the
   first run (Hardening E2) — today the first command is
   `DATABASE_URL=sqlite::memory: cargo run --bin maidan-server`.

**One-click artifacts** (fragmented standard; ship only after M.0):
- Cursor: `cursor://anysphere.cursor-deeplink/mcp/install?name=&config=` plus base64 JSON. Config key `mcpServers` in `~/.cursor/mcp.json`.
- VS Code: `vscode://mcp/install?` plus URL-encoded JSON, NOT base64. Workspace `.vscode/mcp.json` uses `servers`, not `mcpServers`.
- Claude Desktop: `.mcpb` bundle (zip + manifest.json). No reliable web deeplink.

MCP spec 2026-07-28 Streamable HTTP: GET stream + protocol-level sessions **removed**. Verify Maidan MCP against this before shipping a pack. Remote MCP is Streamable HTTP + OAuth 2.1, not stdio. Do not wrap Maidan as an ACP agent inside Cursor.

**Hero demo script (what the GIF should show later)**

1. Human (or Cursor) posts "Ship a health endpoint" in `#demo`.
2. A **static DAG** (thread dependencies + `wait_for_result`), not a
   skill "router." Skills (230-233) are an **AND-gate on
   `claim_next_thread`** (required skills vs member skills). They do
   not dispatch or fan out.
   - researcher `claim_next_thread`, posts findings, `set_thread_result`
   - implementer `wait_for_result`, posts a fake patch / artifact
   - reviewer `wait_for_result`, `request_approval`
3. Operator hits approve in `/ui` (or Slack, after Bet 1).
4. Thread reaches terminal state; digest/notification fires (237-257).

That is *the* product in two minutes. Until this exists, README is a
capability list. Seed **offline**: scripted agents (no LLM) that still
exercise `claim_next_thread` / DAG / result. Optional `--with-llm` later.

### MVP vs later

**MVP**

- M.0 = J3: MCP `2026-07-28` in `SUPPORTED_PROTOCOL_VERSIONS` + honest Streamable HTTP
- `examples/cursor-mcp.json` + `examples/claude-desktop.json`
- `examples/demo-dag/` seed (SQL or `maidan-cli` script) + Integration.md
  "10-minute hero"
- One recorded GIF *after* it works (star-tax; parked until you reopen stars)

**Later**

- `maidan demo up` compose profile
- ACP worker as the "implementer" (ties to Bet 1 later slice)
- Published MCP registry listing (when/if they take third-party servers)

### Risks

- Demo that needs OpenAI keys on first run will bounce. Seed **offline**.
- Don't make the demo depend on `/ui`. Cursor-only path must work.
- A deeplink that claims 2026-07-28 against `SUPPORTED_PROTOCOL_VERSIONS = ["2024-11-05"]` will bounce in current IDEs and look unfinished.
- Do not grow `catalog.rs`. Subset is the product.

### Suggested slices

| Cluster | Scope |
|---------|--------|
| **M.0** | **= J3.** Required MCP `2026-07-28` upgrade (not a 2024 freeze). Then deeplinks. |
| M.1 | `examples/` MCP snippets + Integration.md 10-minute path (hero subset only) |
| M.2 | Offline DAG seed (three scripted agents, no LLM, using existing hero tools) |
| M.3 | `maidan demo` compose profile |

---

## Bet 3 — Thin client SDKs (TypeScript + Python)

**Priority:** medium. Unlocks Bet 1 (Slack adapter in TS is natural) and every integrator who will not speak REST from curl.

### Problem

Today the client is REST + WS + MCP. Integrators copy curl from Integration.md. A 200-line typed client is the difference between trying it and shipping a bot.

There is still **no** TS or Python package. OpenAPI is real (`utoipa`,
`openapi/paths/api.rs` 844 lines) so types are feasible — generating
the whole surface is not. `catalog.rs` is a kitchen sink; the SDK must
not become one.

### Design

Not a full generated OpenAPI monster on day one. Not a CrewAI clone.
**Wrap REST + WebSocket.** Map orchestration helpers onto tools that
**already exist**; do **not** invent `Crew.kickoff` (or any in-process
multi-agent runtime) inside the client. Maidan is the runtime. The SDK
is a typed speaker.

Two packages, TS first:

- `@maidan/client` (TS, `fetch` + WebSocket)
- `maidan` (PyPI, httpx + websockets) — later

**Pin to a protocol / OpenAPI freeze.** Do not generate from a moving
spec while 270 is still adding import routes. Freeze a **7-method
subset** (MVP) and cap the client at **15 methods** even later. Live
`GET /openapi.json` is the contract; `api.rs` is large enough that an
unscoped codegen will drag in operator/admin surface.

Surface, in order (map to existing names):

1. `Client(base_url, token)`
2. `workspaces.create` | `get` — there is **no** `GET /workspaces` list
   in OpenAPI (`POST /workspaces`, `GET /workspaces/{id}`). Do not invent `workspaces.list`.
3. `channels.list` | `create` (`GET/POST /workspaces/{wid}/channels`)
4. `threads.create` | `get` | `transition`
5. `messages.post` / `messages.list`
6. `threads.claim_next_thread` → REST/MCP `claim_next_thread`.
   `wait_for_result` / `wait_for_mention` / `wait_for_ready` /
   `wait_for_notification` are **MCP live-wait tools**, not REST.
   SDK wait helpers must wrap MCP or WS subscribe (`message_posted`,
   `thread_result_set`), not a made-up REST long-poll.
7. Artifact upload (presign + PUT) as of Clusters 175-178

Method 8 (after 270): workspace import. Do not start C.2 while 270 is
still moving that resource.

Auth: pass the capability-scoped token. Do not mint tokens in the SDK.
Admin token minting stays on the operator side. `token:admin` exists
(`TOKEN_ADMIN` in `maidan-auth`); the client must never request it by
default.

Codegen: if OpenAPI / utoipa is current, generate **types** for the
frozen subset and keep a thin hand-written client. If the spec is
stale, do not generate from a lie. Fix the spec first or hand-write
the 7 methods.

MVP: TS only, methods 1–6, ≤15 methods, pinned to a tagged OpenAPI
rev (v268 client speaks 268; bump when 270 lands).
Later: Python, broader codegen, A2A helper, Slack-bridge package that
**uses** this SDK (Bet 1 consumes it, does not duplicate HTTP).

Risks:

- A stale SDK is worse than none. Pin SDK releases to Maidan minor tags. CI: SDK smoke against compose or a nightly.
- Do not start this while 270 import API is still moving the workspace resource.
- Do not wrap MCP as the primary SDK transport. MCP is the IDE pack (Bet 2). The SDK is REST+WS so a Slack adapter and a bot do not need an MCP host.
- Do not add a `kickoff()` that runs agents in-process. That would make Maidan look like CrewAI with extra steps.

Slices: C.1 inventory OpenAPI and freeze 7 methods; C.2 TS client + example bot (uses `claim_next_thread` / `wait_for_result`); C.3 PyPI `maidan`.

---

## Bet 4 — Durable email retry queue

**Priority:** correctness debt, not a star bet. Do it if you claim notifications you can bet on. Skip if email remains nice-to-have.

Cluster 249 shipped SMTP as best-effort. Program C built notification/digest center (237-257). Operators will assume email means the message left the box. It does not, on 5xx / timeout / DNS blip.

### What is already there (do not rebuild)

- **SMTP exists:** `crates/maidan-server/src/mail.rs`, lettre,
  config-gated `MAIDAN_SMTP_HOST` + `MAIDAN_SMTP_FROM`.
- **It is wired:** `notification_router.rs` `tokio::spawn`s
  `deliver_notification_email` after inserting the notification row.
  Comment on the spawn: best-effort, a failure is logged + metered,
  **never retried**; "a durable retrying queue is a follow-up."
- **Presence skip (253)** and **digest mode (255)** already decide
  *whether* to send. Bet 4 does not invent those policies.
- **Stale lie:** `mail.rs` module docs still say "Not wired into the
  notification router yet." Fix that in **E.1** (same PR as the table,
  or Hardening P0 if you touch docs first).

### What `maidan_outbox` is (do not reuse it as a job queue)

`maidan_outbox` + `outbox_relay.rs` is the **event-bus transactional
outbox** (Clusters 205 / 84). Schema is bus events:

```sql
-- migrations/postgres/0013_outbox.sql (then 0014 quarantine)
maidan_outbox (id, log_id → maidan_events, created_at, published_at, attempts)
```

The relay publishes `BusEnvelope`s after commit. Attempts + quarantine
exist. `list_pending` is a bus drain, not a generic worker. **Do not**
say "reuse the outbox worker" as if it accepted arbitrary jobs. A mail
row is not an event-log row.

### Design

New **`mail_outbox`** (or a job kind that is **not** `maidan_outbox`
events) + a worker **modeled on** `outbox_relay` — same operational
shape, different payload:

- Table: `maidan_mail_outbox(id, notification_id, to, payload, attempt,
  next_attempt_at, last_error, state, quarantined_at)`.
- **Same-tx enqueue** as the notification row (so a crash between
  notify and enqueue cannot drop mail). If SMTP is unconfigured, do
  not enqueue.
- Worker: `FOR UPDATE SKIP LOCKED` claim (the pattern `threads::claim_next` /
  digest / scheduler already use for concurrency; `outbox_relay` has
  attempts + quarantine — steal both), exponential backoff
  (1m, 5m, 25m, dead-letter), `max_attempts` then quarantine.
- **Classify SMTP outcomes:** 4xx / bad address → permanent, mark
  dead, surface in `/ui` center. 5xx / timeout / DNS → retry.
  Do not retry 4xx.
- Metrics: `maidan_mail_outbox_pending`, `maidan_mail_outbox_dead`
  (do not overload `maidan_outbox_*` bus metrics).
- **E.1 also:** rewrite `mail.rs` module docs to match the router
  (wired, best-effort until this bet ships retry).

1–2 clusters if you copy the relay loop; 2–3 if you also want a
notify-nudge. Not "1–2 if the outbox worker is generic" — it isn't.

MVP: persist + retry 5xx + dead-letter + metric + honest module docs.
Later: batching/suppression (do not bother with open/click). Presence
and digest stay in the router; the mail worker only sends what the
router already decided to send.

Slices: E.1 table + same-tx insert + fix `mail.rs` module docs; E.2
worker + SKIP LOCKED + 4xx/5xx classify + backoff + metrics +
Production.md.

---

## Bet 5 — Pack-and-prove leftovers (not features)

Still the right next public moves after 270-272, but not expansion features. They live in [Pre-Public Hardening.md](Pre-Public%20Hardening.md) and [Path to Impressive.md](Path%20to%20Impressive.md):

- README tone, badges, docker-or-binary **before** `cargo run` (today the first command is `DATABASE_URL=sqlite::memory: cargo run --bin maidan-server`; `docker compose --profile full` is later), topics, homepage URL
- Human GitHub release notes (stop shipping auto PR titles as release notes)
- `examples/` (overlaps Bet 2; does not exist today)
- Types/comment residue (**765** Cluster/PR matches in `crates/*.rs`; `models.rs` 33 Cluster refs)
- Module splits (monster files, `wc -l` 2026-08-25):

| Lines | Path |
|------:|------|
| 2230 | `crates/maidan-mcp/src/server.rs` |
| 1695 | `crates/maidan-store/src/postgres/mod.rs` |
| 1532 | `crates/maidan-types/src/models.rs` |
| 1455 | `crates/maidan-store/src/sqlite/mod.rs` |
| 1159 | `crates/maidan-store/tests/event_log.rs` |
| 1057 | `crates/maidan-store/src/store.rs` |
| 1037 | `crates/maidan-server/tests/ws_subscribe_e2e.rs` |
| 989 | `crates/maidan-mcp/src/tools/catalog.rs` |
| 961 | `crates/maidan-server/tests/mcp_streamable_e2e.rs` |
| 844 | `crates/maidan-server/src/openapi/paths/api.rs` |

- Evidence.md / coverage story (`COVERAGE_MIN_LINES` is 40%)
- CONTRIBUTING/SECURITY pre-release language
- Stale `mail.rs` module docs (see Bet 4 / Hardening P0)
- **Performance / load / optimization** — Hardening **H**, not a product
  bet. Harnesses exist (`scripts/loadgen.sh`, criterion benches). The
  work is Postgres baseline, agent-shaped mix (MCP/WS/`claim_next_thread`),
  and measured opts only. Production.md still claims load is uncovered.
- **Provider matrix** — Hardening **I**, not a third database. Two
  dialects (Postgres + SQLite) times many hosts (Neon/RDS/Supabase,
  MinIO/R2, Ollama/OpenAI-compatible, OIDC). `docs/Providers.md` then
  recipes. Do not add MySQL/Mongo/Pinecone.

Star-tax (parked): GIF, logo, OG image, GitHub topics, homepage field. Reopen only when you un-hold stars.

---


## Bet 6 — Git projector (GitHub first, then GitLab / Gitea)

**Priority:** same *shape* as Bet 1 (Slack): a front door into Maidan, not
a second product. **Sequence:** after Bet 2 (pack) and **after or
beside** Slack MVP — share the bridge tables. Do not build this so you
can announce "we are Copilot."

### Problem / who cares

Engineers already live in the forge. In 2026 that means:

- **GitHub Copilot coding agent** (cloud agent): assign an issue, it
  clones in Actions, opens a PR. Lives *in GitHub*.
- **Copilot code review**: MCP + `SKILL.md` on the PR. Lives *in GitHub*.
- **GitLab Duo Agent Platform**: `@duo-developer` on issues/MRs. Lives
  *in GitLab*.
- **Official [github/github-mcp-server](https://github.com/github/github-mcp-server)**
  (~32k stars): how Cursor/Claude talk *to* GitHub (issues, PRs, check
  runs). Not a webhook listener.
- OpenTag already does GitHub issue comments as a second front door
  next to Slack.

Maidan has DAG, leases, `claim_next_thread`, capability tokens, and A2A.
It has **zero** forge I/O (no GitHub App, no GitLab webhook, no check
run). Generic outbound webhooks can *leave* Maidan; nothing maps
`issues` / `pull_request` / `Note Hook` *into* a thread.

The job is the Slack job on a different glass: **issue/PR/MR mention →
Maidan thread → agent work → comment (and optional Check Run)**. The
forge stays the code host. Maidan stays the agent workplace.

### 2026 market evidence

- Copilot cloud agent + code review MCP GA (Jul 2026) made "agent on the
  PR" the default GitHub story. Competing by cloning repos and opening
  PRs is how you become a worse Copilot.
- GitLab's third-party/external agents are mention/assign on issue/MR,
  then a comment or a branch. That is a projector API, not a reason to
  embed Duo.
- Gitea/Forgejo still lack a native agent platform; they speak
  GitHub-ish webhooks. Self-hosters will ask. Bitbucket / Azure DevOps
  are later.
- Cursor Origin is a source-control product. Only add it if David
  actually uses it as a forge; do not guess GitHub slugs from Origin
  slugs.

### Design (Maidan-shaped)

**Reuse Bet 1's bridge**, do not invent a second inbox:

- `maidan_bridge_bindings` — `provider` enum: `slack` | `github` |
  `gitlab` | `gitea` (Forgejo uses `gitea`). Installation id + repo/project
  + workspace/channel mapping.
- `maidan_bridge_inbox` — delivery id unique per provider (GitHub
  `X-GitHub-Delivery`, GitLab `X-Gitlab-Event-UUID` / idempotency key).
  Ack fast; work async. Same echo/retry tests as Slack.

**Ingress (MVP, GitHub App):**

- Events: `issues` (opened), `issue_comment` (created, mention of the
  app), `pull_request` (opened — optional, default off),
  `pull_request_review_comment` later.
- Verify HMAC (`X-Hub-Signature-256`). 10-second timeout = retries;
  ack in <2s like Slack.
- Mention/assign-to-the-app only. **Ambient default OFF** (do not open
  a Maidan thread for every PR in the org).
- Map: one GitHub issue/PR → one Maidan thread (stable external id).
  Comments after that append. Reuse `post_message_with_event`.

**Egress (MVP):**

- Final `ThreadResultSet` → issue/PR comment (permalink back to Maidan).
- Optional **Check Run** on the head SHA: `queued` when claimed,
  `in_progress` while DAG running, `completed`/`failure` on result.
  This is the Maidan-shaped bit GitHub MCP cannot do for *our* agents.
  Permissions: `checks:write`, `issues:write`, `pull_requests:write`,
  `metadata:read`.

**GitLab (R.6):** project webhook (Note + Issue + Merge Request). Post
notes. No Checks API; use a pipeline comment or commit status if someone
asks. Same thread mapping.

**Gitea/Forgejo (R.7):** treat as GitHub-shaped payloads where they
match; recipe, not a third implementation, until a payload diverges.

**Agents talk to git how?** They keep using **GitHub MCP** (or `gh` /
`glab`) for diffs, files, and review comments. Maidan does **not**
reimplement `github-mcp-server`. Document "add both MCP servers":
Maidan for the workplace, GitHub MCP for the repo. Copilot cloud
agent can even be pointed at Maidan MCP (read-only in code review) —
that is a recipe (J8-shaped), not Maidan-core.

**Secrets:** same keyring as Slack/federation (`FEDERATION_ENCRYPTION_KEY`).
GitHub App private key + installation tokens (1h). No PATs in prod, no
stolen session cookies.

### MVP vs later

**MVP (R.1–R.4, 2–4 clusters, GitHub only)**

1. Shared bridge tables with `provider` (or GitHub-only columns if Slack
   S.1 has not landed — then migrate to shared).
2. GitHub App webhook endpoint + signature + inbox.
3. Issue opened-by-app-mention / `issue_comment` @app → thread.
4. Result → comment + Production/Integration docs + App manifest.

**Later**

- R.5 Check Run projector
- R.6 GitLab webhook adapter
- R.7 Gitea/Forgejo recipe
- R.8 `workflow_run` / pipeline failure → thread (noisy; opt-in)
- Review-comment inline replies
- Cursor Origin
- Bitbucket / Azure DevOps
- Opening PRs / pushing commits **as Maidan** (never; that's Copilot/ACP)

### Risks / non-goals

- **Non-goal:** replacing GitHub/GitLab. Non-goal: a SWE agent that
  clones, commits, and opens PRs. Non-goal: wrapping the GitHub REST
  API as 40 more MCP tools.
- **Echo loops:** ignore comments from the App user. Tests before any
  public claim (same as Slack).
- **Retry duplication:** GitHub retries webhooks; inbox uniqueness is
  the product.
- **ToS / retention:** issue bodies in Maidan obey Cluster 186 retention.
- **Don't** use a personal PAT for an org bot. Don't scrape
  `github.com` HTML.

### Suggested slices

| ID | Scope |
|----|--------|
| **R.1** | Bridge binding+inbox with `provider` (share with Slack S.1 if both exist) |
| **R.2** | GitHub App HTTP webhook + HMAC + ack + enqueue |
| **R.3** | Mention/issue → thread/message (`post_message_with_event`) |
| **R.4** | Result → issue/PR comment + App manifest + docs |
| **R.5** | Check Run queued/in_progress/completed from claim/DAG/result |
| **R.6** | GitLab webhook + notes |
| **R.7** | Gitea/Forgejo recipe on the GitHub-shaped path |

---
## Explicitly do not chase

| Temptation | Why not |
|------------|---------|
| Loadgen as a required CI p99 gate | Cluster 198 is `#[ignore]`d on purpose (runner hardware). Nightly error-rate only (Hardening H3). |
| Redis / batched `pg_notify` for speed | Redis: measure first (H4). Batched NOTIFY was **declined** (delivery-core risk). |
| A third database engine (MySQL, Mongo, Dynamo) | `Store` is 228 methods x two backends already. LISTEN, pgvector, LSN replicas are Postgres. Hosts that speak Postgres (Neon, RDS, Aurora, Supabase) are a **docs/recipe** job (Hardening I), not a new crate. |
| Native embedding SDKs (Voyage, Anthropic, Bedrock) | `openai-compatible` already covers every `/v1/embeddings` host. Add a recipe, not a protocol. Chat LLMs stay in the agent. |
| Native GCS / Azure Blob | S3-compatible covers MinIO, AWS, R2. Native only if a user is blocked. |
| Pinecone / Qdrant as a required store | Already have pgvector + openai-compatible embeddings. Optional adapter later. |
| Slack huddles / emoji-as-product / Slack Connect | Front door is mentions + cards. Recreating Slack is how /ui almost went wrong. |
| SPA rewrite of /ui | Operator/HITL surface. Browser e2e only if humans live there. Ten `ui_*` files; `ui_js_contract` is static; no Playwright — keep it that way unless the north star flips. |
| SAML-in-core | OIDC exists. SAML is a later enterprise checkbox. |
| Embedding an LLM in Maidan | Substrate. The teammate is a connected agent (MCP/A2A/ACP). |
| ACP as a replacement for A2A | **Two ACPs:** IBM Agent Communication Protocol merged into A2A (2025-08-29). Zed Agent Client Protocol is editor↔coding agent. Adapter for Zed, never a protocol swap. |
| A Maidan-native agent protocol | MCP + A2A + REST is the 2026 AAIF stack. Hardening J is honesty/alignment, not a fourth wire. |
| IBM ACP / BeeAI native | Dead. Use A2A. |
| Native AG-UI / CopilotKit runtime | WS + `/ui` already present events. Only if north star flips to a React product. |
| A2A gRPC or GraphQL gateway | JSON-RPC + OpenAPI cover public agents and IT. Bindings on demand. |
| ANP / AP2 / A2UI / MCP Apps as required | Watch lists. Not adoption blockers. |
| Stealth Slack (session-cookie MCP) | Stars with a ToS smell. Use a real Slack app. |
| Reimplementing `github-mcp-server` | Official server is how agents talk *to* Git. Maidan projects events *from* Git. Add both. |
| Maidan-as-Copilot (clone, commit, open PRs) | That's GitHub's coding agent / ACP workers. We map issues to threads. |
| GitLab Duo / Copilot review as a protocol | Mention/webhook projectors. Don't embed their runtimes. |
| Bitbucket / Azure DevOps / Origin as MVP | GitHub first, GitLab second, Gitea recipe. Origin only if David uses it. |
| Second memory product (MEMORY.md files) | Threads + artifacts + results are memory. |
| Federated search / another vector DB | Search replica routing (271–272) first — **other agent**; then stop. |
| `Crew.kickoff` in-process runtime | Bet 3 wraps REST+WS. Maidan already *is* the orchestrator. |
| Reusing `maidan_outbox` as a mail/job queue | Bus-event outbox. Mail gets its own table. |
| Adding MCP tools for the hero pack | Subset `catalog.rs`. The tools exist. |
| Search replica routing as an expansion bet | `PostgresSearch` has its own `PgPool`. 271–272 is the other agent's job. |

---

## Recommended sequence after 270-272

0. **Hardening P0 + H1/H5 + I1 + J1** (tone, loadgen baseline, **`docs/Providers.md`**, **`docs/Protocols.md`**) **can run in parallel** with the other agent's 270–272. Not an expansion bet.
1. **J3 / Bet 2 M.0** — required MCP `2026-07-28` upgrade (stateless Streamable HTTP). Then **M.1** `examples/` / 2026 snippets + **M.2** offline DAG. Do not ship a 2024-only pack. Do not depend on `/ui`.
2. **Bet 3** TS client (C.1 freeze → C.2). Unblocks Bet 1 without a second HTTP stack. ≤15 methods, REST+WS, map to `claim_next_thread` / `wait_for_result`.
3. **Bet 4** mail retry (E.1–E.2) **if** claiming reliable notifications; skip if email stays nice-to-have. New `mail_outbox`, not `maidan_outbox`.
4. **Bet 1** S.1–S.4 Slack MVP as a **projector**, not the product (HTTP Events, mention-only, final-message first).
5. Hardening H2–H4 (agent-shaped load mix, then measured opts) + residue/module splits. Star-tax when you reopen stars.
6. Bet 1 S.5–S.6 (native streams + HITL) after Slack MVP is boringly stable.
7. **Bet 6 R.1–R.4** GitHub projector (share bridge tables with Slack). GitLab/Gitea and Check Runs after the GitHub comment loop is boring.
8. Public cut / spreading the word: [Launch.md](Launch.md) — after Hardening P0 + L1–L4, not as a reason to start Slack/Git early.

If you can only do one expansion: Bet 2 is the someone-stars-it-this-month play (pack + hero). Bet 1 (Slack) and Bet 6 (Git) are category *projectors*, not new products. Do 2 then 3 then 1 or 6 (pick the glass your users already stare at). Star-tax stays parked until [Launch.md](Launch.md) tag day.

Do **not** add workspace import, A2A content-to-parts, or search replica routing as expansion bets. Those are the in-flight optional-deferrals sweep (269–272 / already shipped 267). This file stays outside that ladder.

---

## Sources

- Anthropic, Introducing Claude Tag, 2026-06-23: https://www.anthropic.com/news/introducing-claude-tag
- Slack Events API (ack/retry): https://docs.slack.dev/apis/events-api.md
- Slack HTTP vs Socket Mode (Marketplace requires HTTP): https://docs.slack.dev/apis/events-api/comparing-http-socket-mode
- Slack native chat streaming (7 Oct 2025): https://docs.slack.dev/changelog/2025/10/7/chat-streaming
- Slack chat.startStream: https://docs.slack.dev/reference/methods/chat.startstream
- Slack Socket Mode: https://docs.slack.dev/apis/events-api/using-socket-mode
- Slack Web API rate limits: https://docs.slack.dev/apis/web-api/rate-limits/
- Agent Client Protocol: https://agentclientprotocol.com/
- MCP spec 2026-07-28: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- A2A Linux Foundation one-year (150+ orgs, v1.0): https://www.linuxfoundation.org/press/a2a-protocol-surpasses-150-organizations-lands-in-major-cloud-platforms-and-sees-enterprise-production-use-in-first-year
- IBM ACP merged into A2A (2025-08-29): https://lfaidata.foundation/communityblog/2025/08/29/acp-joins-forces-with-a2a-under-the-linux-foundations-lf-ai-data/
- [Protocols.md](Protocols.md) — inventory + 2026 layer map
- GitHub Copilot coding agent: https://docs.github.com/copilot/concepts/agents/cloud-agent/about-cloud-agent
- Copilot code review MCP GA (2026-07-29): https://github.blog/changelog/2026-07-29-copilot-code-review-agent-skills-and-mcp-now-generally-available/
- Official GitHub MCP server: https://github.com/github/github-mcp-server
- GitLab Duo external agents: https://docs.gitlab.com/user/duo_agent_platform/agents/third_party/
- OpenTag: https://github.com/amplifthq/opentag
- [Launch.md](Launch.md) — public cut + announce
- Star counts in the market snapshot table: GitHub API, 25 Aug 2026.
- Maidan tree audit (this file's constraints): local `rg`/`wc` 2026-08-25. Crates, MCP `SUPPORTED_PROTOCOL_VERSIONS`, `mail.rs` vs `notification_router.rs`, `PostgresSearch` pool, monster-file line counts, Cluster residue 765.
- Maidan clusters this doc assumes: 171, 173-178, 186, 189-192, 194/267, 217-236, 237-257, 249, 253, 255, 266-272 (all shipped; see `docs/Retros/` on GitHub, e.g. the Cluster 269.0–272.0 retros for the workspace-import + search-replica work)

---

## Changelog of this file

- 2026-08-25 (Git + launch): Bet 6 Git projector (GitHub App → thread → comment/Check Run; GitLab/Gitea later; do not reimplement GitHub MCP). [Launch.md](Launch.md) for production-ready extras L1–L6, public-preview cut, Show HN. GitHub-issue ingress removed from Bet 1 leftovers.
- 2026-08-25 (handoff audit): added [Handoff.md](Handoff.md) as the session start page (master IDs + try-out matrix). I1/J1 marked written. mdBook SUMMARY + `book/sync-docs.sh` include the pack.
- 2026-08-25 (afternoon, later): protocol research pass. Added Hardening J / [Protocols.md](Protocols.md) as the "whatever they already speak" track (MCP+A2A+REST, not a fourth protocol). IBM ACP called dead; Zed ACP stays adapter-only; AG-UI/gRPC/GraphQL/ANP on the do-not-chase table. Sequence 0 includes J1.
- 2026-08-25 (afternoon): re-audit against the local tree. Added "Codebase constraints"; boxed 269–272 as the other agent's ladder; named `claim_next_thread` / EventKind wire names; Bet 2 M.0 protocol honesty; Bet 3 REST+WS freeze; Bet 4 `mail_outbox` (not `maidan_outbox`); monster-file counts + residue 771; sequence 0 = Hardening P0 in parallel.
- 2026-08-25: first cut after v267-v268 shipped and 269 import store in flight. Rescored prior recs; researched Slack/ACP/Claude Tag; wrote four expansion bets plus anti-catalog. Corrected Slack egress to native chat.startStream (7 Oct 2025); added live GitHub star snapshot; Marketplace HTTP Events requirement; MCP one-click artifacts.
