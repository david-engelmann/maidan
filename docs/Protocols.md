# Integration protocols

**Audience:** someone plugging Maidan into an existing agent stack (Cursor, Claude Desktop, a Python/TS agent, another org's A2A agent, n8n, Slack).

**Companion:** [Providers.md](Providers.md) is *where it runs* (Postgres host, S3, OIDC). This page is *how it talks*. Execution checklist: Hardening **J**. Feature packs that sit on top (one-click MCP, thin SDK, Slack projector) live in [Expansion Bets.md](Expansion%20Bets.md).

Snapshot: 2026-08-25. Code facts from the local tree (`SUPPORTED_PROTOCOL_VERSIONS`, `POST /a2a/v1/rpc`, Agent Card). Market facts from AAIF / Linux Foundation / MCP spec `2026-07-28` / A2A v1.0. Re-scan before you quote numbers in a blog post.

**MCP `2026-07-28` shipped (Hardening J3, Clusters 300–303).** The server negotiates the current
`2026-07-28` revision — stateless Streamable HTTP (no `Mcp-Session-Id`) + SEP-2243 `Mcp-Method`/`Mcp-Name`
routing headers — and still accepts `2024-11-05` for older clients. See [Required protocol upgrades](#required-protocol-upgrades).

---

## The 2026 stack (do not pick a winner)

These are **layers**, not alternatives. Pickaxe / AAIF / Linux Foundation all say the same thing in 2026: MCP won tools; A2A won peers; a UI protocol is emerging on top.

| Layer | Protocol | Job | Analogy |
|-------|----------|-----|---------|
| Capability | **MCP** (Anthropic → AAIF) | Agent ↔ tools / data | USB-C |
| Coordination | **A2A** (Google → Linux Foundation) | Agent ↔ agent tasks | Phone line |
| Presentation | **AG-UI** (CopilotKit) or Maidan WS/`/ui` | Agent ↔ human surface | Screen |
| Existing IT | REST + OpenAPI, WebSocket, webhooks, OIDC, Prometheus/OTLP | The stack they already run | Plumbing |

IBM's **Agent Communication Protocol** (BeeAI) **merged into A2A** on 2025-08-29. Do not implement it. Zed's **Agent Client Protocol** is a *different* ACP (editor ↔ coding agent, LSP-shaped). OpenTag uses that one. Maidan optionally *dispatches* an ACP worker; it must not become Maidan's native workspace protocol.

**Start with MCP.** Add A2A when a second autonomous agent must discover and delegate. Do not invent a fourth agent protocol.

---

## What Maidan already speaks (code, 2026-08-25)

One model, one capability map, four primary transports plus the IT surfaces.

| Surface | Where | Status | Honest caveat |
|---------|-------|--------|----------------|
| REST + OpenAPI 3.0 | `GET /openapi.json`, utoipa | Production | No `workspaces.list`. Create via `POST /workspaces`. Hero bootstrap is REST/CLI, not MCP. |
| MCP JSON-RPC | `POST /mcp` | Production, **negotiates `2026-07-28`** (+ `2024-11-05`) | `SUPPORTED_PROTOCOL_VERSIONS = ["2026-07-28","2024-11-05"]`, default `2026-07-28`. `POST /mcp` is stateless (JSON-RPC in/out). |
| MCP Streamable HTTP | `POST/GET/DELETE /mcp/streamable` | Production; **`2026-07-28` stateless** (+ `2024-11-05` session) | A `2026-07-28` POST lands cold: single JSON-RPC response, no `Mcp-Session-Id`, optional SEP-2243 `Mcp-Method`/`Mcp-Name` headers. A `2024-11-05` POST keeps the SSE-session model (first POST opens SSE + `Mcp-Session-Id`; GET opens server→client notifications). Live-wait rides `GET /mcp/stream`, not a 2026 POST session. |
| MCP SSE (legacy-shaped) | `GET /mcp/stream`, `GET /mcp/notifications` | Production | Fine for Maidan live-wait. HTTP+SSE is deprecated in the MCP spec (SEP-2596); migrate *clients* toward Streamable HTTP, not a third Maidan transport. |
| MCP stdio | `maidan mcp-stdio` | Production | The desktop-client path (Claude Desktop / local Cursor). Same JSON-RPC, SQLite or Postgres. |
| WebSocket | `GET /ws/subscribe` | Production | Resumable cursors, capability `event:subscribe`. This is Maidan's agent↔UI live path. |
| A2A JSON-RPC v1.0 | `POST /a2a/v1/rpc`, `POST /a2a/v1/events` | Production subset | Methods: SendMessage, SendStreamingMessage, GetTask, SubscribeToTask, tasks/cancel, resubscribe, pushNotificationConfig get/set. **Egress parts are text-only** (v267). **gRPC binding is partial** — the `A2AService` exposes `get_task`/`cancel_task`/`list_tasks` only; `SendMessage`, push configs, and streaming are JSON-RPC/REST only. |
| A2A Agent Card | `GET /.well-known/agent-card.json` | Present, **custom schema** | Fields: `name`, `version`, `protocol_version`, `rpc_url`, `ingress_url`, `capabilities[]`. Spec v1.0 wants `supportedInterfaces[]` (`JSONRPC` / `GRPC` / `HTTP+JSON`), skills, auth. A strict A2A SDK may reject this card. |
| Federation card | `GET /.well-known/maidan.json` | Production | Maidan-to-Maidan, not A2A. |
| Outbound webhooks | `/workspaces/:wid/webhooks`, mention-webhook | Production | Signed POSTs of event envelopes. The n8n / Zapier / Make path. |
| Slash commands | `/workspaces/:wid/slash-commands` | Production | HTTP callbacks, Slack-shaped. |
| FSM hooks | `fsm_hooks` | Production | Thread state machine → HTTP. |
| Human auth | OIDC discovery | Production | Session cookies for `/ui`. Agents use capability bearers. |
| App OAuth | `/oauth/app/token` | Production | Installed apps, not MCP resource-server OAuth (RFC 8707). |
| Metrics | `GET /metrics` + OTLP smoke in CI | Production | Prometheus text. Plug into the scrape they already run. |

MCP tool count is **78**. There is **no** MCP create workspace / channel / thread / member. An MCP-only agent cannot bootstrap a hero demo. Seed via REST or CLI, then MCP for claim / wait / post.

---

## Who shows up with which protocol

| They already run | Point them at | Do not |
|------------------|---------------|--------|
| Cursor, Claude Desktop, VS Code, Claude Code, ChatGPT connectors | MCP **`2026-07-28`** (shipped) — `POST /mcp` / Streamable HTTP / stdio; older clients may still request `2024-11-05`. | — |
| A Python / TS agent they wrote | REST + WS, or MCP if they already have an MCP client. Thin SDK is Bet 3. | An in-process `Crew.kickoff`. Maidan *is* the orchestrator. |
| LangGraph / CrewAI / OpenAI Agents SDK | Recipe on REST+WS (or MCP tools). Those frameworks speak MCP as of 2026; they do not need a Maidan-native runtime. | A LangGraph checkpointer inside Maidan. |
| Another vendor's agent (Salesforce, SAP, Bedrock, Foundry) | A2A Agent Card + JSON-RPC. | IBM ACP. It is A2A now. |
| n8n / Zapier / Make / "we have webhooks" | Outbound webhooks + REST. OpenAPI for the REST half. | A GraphQL gateway. |
| Humans in Slack | Bet 1 projector (HTTP Events API). Agents stay on MCP/A2A. | Making Slack the datastore. Socket Mode as Marketplace default. |
| Humans in GitHub / GitLab / Gitea | Bet 6 projector (GitHub App / webhooks). Agents use official GitHub MCP for diffs. | Reimplementing GitHub MCP. Opening PRs as Maidan. Ambient on every PR. |
| Humans in the browser / a React app | Today: `/ui` + WS. Later, *maybe* AG-UI if `/ui` becomes a real product. | Native AG-UI this quarter. CopilotKit is a frontend stack, not a workspace. |
| Coding agent in Zed / JetBrains (OpenTag-shaped) | Optional ACP *adapter*: Maidan thread → spawn ACP agent → result back. | Replacing A2A or MCP with Zed ACP. |
| Observability (Grafana, Datadog, Honeycomb) | `/metrics` + existing OTLP smoke. | OpenTelemetry as a fourth agent protocol. |
| SSO they already pay for | OIDC (Providers.md). | SAML-in-core. MCP-spec OAuth only if remote MCP hosts refuse bearer tokens. |

---

## Market evidence (why this order)

Researched 2026-08-25. Quote the primary sources if you blog; do not inflate.

- **MCP is the default connect story.** Public writeups in 2026 treat it as the de facto agent↔tool standard (Cursor, Claude, ChatGPT, Gemini, JetBrains, Vercel AI SDK). Spec current rev is **`2026-07-28`**: stateless Streamable HTTP, `Mcp-Method` / `Mcp-Name` headers, capabilities on every request `_meta`, sessions gone. Anthropic rolled that rev across Claude products the same day. Maidan has not.
- **A2A is the default peer story.** Linux Foundation, v1.0, 150+ orgs (AWS, Microsoft, Google, IBM, Salesforce, SAP, ServiceNow), cloud embeddings in Azure AI Foundry / Copilot Studio / Bedrock AgentCore. JSON-RPC over HTTP is the common public binding; gRPC and HTTP+JSON are spec bindings, not requirements. GitHub `a2aproject/A2A` ~25k stars (snapshot in Expansion Bets).
- **AAIF** (Agentic AI Foundation, Linux Foundation, Dec 2025) now governs MCP *and* A2A together. Building a private third protocol in 2026 is the anti-pattern those posts keep naming.
- **IBM ACP is dead as a product.** Merged into A2A 2025-08-29. Docs redirect. Mention it only to tell people to use A2A.
- **Zed ACP is real and adjacent.** Editor ↔ coding agent. OpenTag (~1.3k stars) is the Slack-shaped dispatcher. Adapter later, not native.
- **AG-UI** is the emerging agent↔frontend event stream (CopilotKit). Complements MCP/A2A. Maidan already has WS event envelopes. Do not dual-implement a CopilotKit runtime until humans-in-browser is the north star.
- **ANP** (decentralized DID agent marketplace), **AP2** (agent payments), **A2UI** (Google generative UI widgets): watch, do not build.
- **GraphQL / gRPC as Maidan's primary API:** nobody asking for a Slack-shaped workspace leads with GraphQL. A2A's optional gRPC binding is for *A2A*, not a rewrite of `/workspaces`.

---

## Required protocol upgrades

**`2024-11-05`-only MCP is not a shippable state.** Cursor, Claude, and the
2026 SDKs speak **`2026-07-28`**. A pack or public cut that advertises MCP
while `SUPPORTED_PROTOCOL_VERSIONS = ["2024-11-05"]` will bounce modern
clients. Do not "freeze on 2024" as the strategy. Temporary honesty (J2)
until the upgrade lands is not the same as accepting 2024 forever.

| Protocol | Code today (2026-08-25) | Required | ID |
|----------|-------------------------|----------|-----|
| **MCP** | ✅ **`2026-07-28` shipped** (default; `2024-11-05` still accepted). Stateless Streamable HTTP (no `Mcp-Session-Id`), SEP-2243 `Mcp-Method`/`Mcp-Name` headers, live-wait on `GET /mcp/stream`/WS. | Done in Clusters 300–303. | **J3** ✅ (Clusters 300–303) |
| **A2A Agent Card** | Custom `{rpc_url, capabilities[]}` | Spec v1.0 `supportedInterfaces` (`JSONRPC`) | J4 |
| **A2A parts** | Egress text-only (v267) | File/data parts when artifacts exist | J5 |
| MCP OAuth (RFC 8707) | Capability bearers | Only if a real 2026 host refuses bearer after J3 | J6 |

### J3 — MCP `2026-07-28` (do this; do not sticker it)

Spec: https://blog.modelcontextprotocol.io/posts/2026-07-28/

What has to change in *this* tree (`maidan-mcp` + `mcp_streamable.rs`):

1. `SUPPORTED_PROTOCOL_VERSIONS` includes **`2026-07-28`** and that rev is
   what `initialize` returns to current clients.
2. Streamable HTTP POST carries **`Mcp-Method`** and **`Mcp-Name`** (SEP-2243)
   so a gateway can route without parsing JSON.
3. **Stateless core:** capabilities / protocol version from `_meta` (or the
   headers) on each request. A 2026 client must not need `Mcp-Session-Id`.
4. **GET `/mcp/streamable` + protocol-level sessions are not 2026.** Keep
   Maidan live-wait as `GET /mcp/stream` / WS / `wait_for_*` tools. Do not
   tell a 2026 client that GET-session *is* Streamable HTTP 2026.
5. Tests: `initialize` with `2026-07-28` succeeds; a Cursor-shaped client
   that omits a session id can `tools/call`. README/Integration advertise
   2026 **only after** 1–4 are green.

Optional one-release fallback: still *accept* `2024-11-05` initialize from
old stdio clients if it does not revive the session lie. Default and
docs are 2026. **Staying 2024-only is not an option.**

J3 is Hardening (protocol upgrade), not Bet 2. Bet 2 **M.0 is J3**. The
pack (M.1) and public cut wait on it. Do not sneak this into a docs PR.

## Gaps worth closing (Hardening J + existing bets)

J3 shipped (`2026-07-28`, Clusters 300–303). The rest is adapters + honesty. No new native protocol.

| ID | Gap | Size | Notes |
|----|-----|------|-------|
| **J1** | This page | Docs | **Written 2026-08-25.** Keep true when `SUPPORTED_PROTOCOL_VERSIONS` changes. |
| **J2** | ✅ Retired | Docs | Was: "temporary honesty (today 2024-11-05)". No longer needed — J3 shipped (Clusters 300–303); README/Integration now advertise `2026-07-28`. |
| **J3** | ✅ MCP `2026-07-28` **shipped** | Done | Clusters 300 (negotiation) → 301 (stateless streamable core) → 302 (SEP-2243 routing headers) → 303 (advertise: default flip + card/reference/Integration). `2024-11-05` still accepted. |
| **J4** | A2A Agent Card → spec v1.0 `supportedInterfaces` | Small | Keep JSON-RPC URL. Advertise `protocolBinding: JSONRPC`. Do not add gRPC just to fill the array. Signed JWS cards are enterprise-later. |
| **J5** | A2A file/data parts | Cluster (after 267 text) | Ingress already preserves structured content; egress is text-only. Round-trip files when an artifact already exists. |
| **J6** | MCP OAuth resource-server (RFC 8707) | Spike, then maybe | Remote Claude/Cursor may insist. Today: capability bearers. Implement only if a real host refuses the bearer. Do not replace workspace capabilities with a second ACL. |
| **J7** | Webhook + OpenAPI recipe for n8n/Zapier | Docs | They already work. Show one signed webhook + one REST post. |
| **J8** | LangGraph / CrewAI / Agents SDK recipe | Docs / `examples/` (Bet 2/3) | REST+WS or MCP tools. No in-process runtime. |

**Already covered elsewhere, do not duplicate here:** Slack Events projector (Bet 1), thin TS SDK (Bet 3), MCP `examples/` pack (Bet 2 M.1), create-* MCP tools (no — seed via REST).

---

## Do not chase

| Temptation | Why not |
|------------|---------|
| A fourth agent protocol ("Maidan Protocol") | MCP+A2A+REST is the industry stack. AAIF exists so you do not do this. |
| IBM ACP / BeeAI native | Merged into A2A. |
| Zed ACP as the workspace | Wrong layer. Optional worker adapter. |
| Native AG-UI / CopilotKit runtime | WS + `/ui` already present the events. AG-UI when the north star is a React product. |
| A2A gRPC or HTTP+JSON bindings "for completeness" | JSON-RPC is what public agents speak. Add a binding when a cloud (Foundry/Bedrock) blocks on it. |
| GraphQL gateway | OpenAPI is the IT path. |
| gRPC for `/workspaces` | Same. |
| ANP, AP2, A2UI, MCP Apps as required | Watch lists. Not adoption blockers. |
| MCP HTTP+SSE as a *new* transport | We already have `/mcp/stream`. Spec says migrate to Streamable HTTP. |
| MCP create-workspace tools so an IDE can bootstrap | Hero seed is REST/CLI by design. 78 tools is enough. |
| OpenAI Assistants / Responses as a native wire | Those clients speak MCP now. |
| Teams/Discord as first-class protocols | Slack projector first if any chat bridge. |
| GitHub MCP as Maidan tools | Official server is the repo wire. We ingest webhooks. |
| Replacing capability bearers with only OIDC for agents | Humans are OIDC. Agents are scoped tokens. Keep the split. |

---

## Integrator decision tree

1. **Single agent, needs Maidan tools** → MCP **`2026-07-28`** (shipped; stdio local, stateless Streamable HTTP remote). Older clients may request `2024-11-05`.
2. **Need live events in your own UI** → WebSocket subscribe (or MCP SSE live-wait).
3. **Need to script / generate a client / talk to n8n** → REST + OpenAPI, optionally webhooks.
4. **A second *agent* must delegate to Maidan or vice versa** → A2A JSON-RPC + Agent Card (J4).
5. **Humans already live in Slack** → Bet 1 projector, not a new protocol.
6. **Humans already live in GitHub/GitLab** → Bet 6 projector, not Copilot.
7. **Editor coding agent should work a Maidan thread** → ACP adapter later, not now.

If two of those apply, use two transports. That is the design (README: "one surface, four transports").

---

## See also

- [Integration.md](Integration.md) — start here to actually connect
- [Providers.md](Providers.md) — hosts, not wires
- [Capability Map.md](Capability%20Map.md) — the same ACL on every transport
- [Pre-Public Hardening.md](Pre-Public%20Hardening.md) — section J
- [Expansion Bets.md](Expansion%20Bets.md) — MCP pack, SDK, Slack
- [Path to Impressive.md](Path%20to%20Impressive.md)
- MCP spec `2026-07-28`: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- A2A spec: https://a2a-protocol.org/v1.0.0/specification
- Agent Client Protocol (Zed): https://agentclientprotocol.com/
