# Integration protocols

**Audience:** someone plugging Maidan into an existing agent stack (Cursor, Claude Desktop, a Python/TS agent, another org's A2A agent, n8n, Slack).

[Providers.md](Providers.md) covers *where it runs* — Postgres host, S3, OIDC.
This page covers *how it talks*.

Protocol facts come from the code (`SUPPORTED_PROTOCOL_VERSIONS`,
`POST /a2a/v1/rpc`, the Agent Card) and are current. The market commentary below
was written on 2026-08-25 and ages faster than the code does.

**MCP `2026-07-28` is the default; every revision since `2024-11-05` is
accepted.** `initialize` echoes `2026-07-28`, `2025-11-25`, `2025-06-18`,
`2025-03-26` or `2024-11-05` — whichever the client asks for — and every
revision from `2025-03-26` on is served over the same stateless Streamable HTTP
(no `Mcp-Session-Id`; optional SEP-2243 `Mcp-Method`/`Mcp-Name` headers). Only
`2024-11-05` keeps the SSE-session model. The 2025 revisions matter most: the
official TypeScript SDK 2.0 and the MCP Inspector request `2025-11-25` and do not
accept `2026-07-28`, and until Cluster 412 their handshake with Maidan failed.
`scripts/mcp-inspector.sh` runs the Inspector against a real server to keep it
that way. See [Required protocol upgrades](#required-protocol-upgrades).

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
| REST + OpenAPI 3.1 | `GET /openapi.json`, utoipa | Production | No `workspaces.list`. Create via `POST /workspaces`. Hero bootstrap is REST/CLI, not MCP. |
| MCP JSON-RPC | `POST /mcp` | Production, **negotiates `2026-07-28`, `2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`** | `SUPPORTED_PROTOCOL_VERSIONS`, default `2026-07-28`. `POST /mcp` is stateless (JSON-RPC in/out). |
| MCP Streamable HTTP | `POST/GET/DELETE /mcp/streamable` | Production; **stateless from `2025-03-26` on** (+ `2024-11-05` session) | A POST from any revision `2025-03-26` or later lands cold: one JSON-RPC response on its own POST, no `Mcp-Session-Id`, a notification answered `202`, optional SEP-2243 `Mcp-Method`/`Mcp-Name` headers. Sessions are opt-in: only a `2024-11-05` client — by its `initialize` or its `MCP-Protocol-Version` header — gets the SSE-session model (first POST opens SSE + `Mcp-Session-Id`). `GET` opens server→client notifications. Live-wait rides `GET /mcp/stream`, not a POST session. |
| MCP SSE (legacy-shaped) | `GET /mcp/stream`, `GET /mcp/notifications` | Production | Fine for Maidan live-wait. A stateless `resources/subscribe` is kept in the database, so `GET /mcp/notifications` on any replica delivers it. HTTP+SSE is deprecated in the MCP spec (SEP-2596); migrate *clients* toward Streamable HTTP, not a third Maidan transport. |
| MCP stdio | `maidan mcp-stdio` | Production | The desktop-client path (Claude Desktop / local Cursor). Same JSON-RPC, SQLite or Postgres. |
| WebSocket | `GET /ws/subscribe` | Production | Resumable cursors, capability `event:subscribe`. This is Maidan's agent↔UI live path. |
| A2A v1.0 (JSON-RPC, HTTP+JSON) | `POST /a2a/v1/rpc`, `/a2a/v1/message:send` and the rest of §11 | Production; **checked by the official A2A TCK** in CI (`scripts/a2a-tck.sh`; excluded cases in `scripts/a2a-tck/exclusions.txt`) | Every v1.0 operation: SendMessage, SendStreamingMessage, GetTask, ListTasks, CancelTask, SubscribeToTask, the four push-config operations, GetExtendedAgentCard. Requests send `A2A-Version: 1.0`; a request without it is 0.3 and refused. A `contextId` is a Maidan thread, and the message is posted as the token's member. Parts are text or URL. The **gRPC binding** (opt-in, `MAIDAN_A2A_GRPC_ADDR`) serves the official `lf.a2a.v1.A2AService` from the unmodified v1.0.1 `a2a.proto`, every operation included, and the TCK runs over it too. `POST /a2a/v1/events` is federation ingest, not A2A. |
| A2A Agent Card | `GET /.well-known/agent-card.json` | Production (spec v1.0) | `supportedInterfaces` (`JSONRPC`, `HTTP+JSON`, optional `GRPC`), capabilities, skills, a bearer `securitySchemes` entry; cacheable (`ETag`, `Cache-Control`). The extended card needs a token. |
| Federation card | `GET /.well-known/maidan.json` | Production | Maidan-to-Maidan, not A2A. |
| Outbound webhooks | `/workspaces/:wid/webhooks`, mention-webhook | Production | Signed POSTs of event envelopes. The n8n / Zapier / Make path. |
| Slash commands | `/workspaces/:wid/slash-commands` | Production | HTTP callbacks, Slack-shaped. |
| FSM hooks | `fsm_hooks` | Production | Thread state machine → HTTP. |
| Human auth | OIDC discovery | Production | Session cookies for `/ui`. Agents use capability bearers. |
| App OAuth | `/oauth/app/token` | Production | Installed apps, not MCP resource-server OAuth (RFC 8707). |
| Metrics | `GET /metrics` + OTLP smoke in CI | Production | Prometheus text. Plug into the scrape they already run. |

MCP tool count is **240**. There is **no** MCP create workspace or member. An agent creates a channel with `create_channel` and a thread with `create_thread` (both `workspace:write`). Workspace and member bootstrap stay on REST or the CLI; then MCP for claim / wait / post / `transition_thread`.

Every tool in `tools/list` carries `annotations`: a `title` and explicit `readOnlyHint`, `destructiveHint`, `idempotentHint` and `openWorldHint`, the same on `/mcp`, `/mcp/streamable`, `/mcp/worker` and `/mcp/reviewer`. `openWorldHint` is true only for a tool that itself reaches outside Maidan, such as `post_message` running a slash command's HTTP receiver or `search_messages` calling a remote embedding provider. The reason for each value is in `crates/maidan-mcp/tests/fixtures/tool-annotations.json`, and `tool_annotations_contract` fails when a tool lacks a hint or disagrees with that table.

## MCP discovery and cache hints

`server/discover` answers with no handshake before it: the revisions Maidan
supports (`supportedVersions`), its capabilities, the server instructions, and
`serverInfo` under `_meta["io.modelcontextprotocol/serverInfo"]`. MCP
`2026-07-28` has no `initialize`, so a client that speaks only that revision
reads the instructions here. That discover result, and an `initialize` that
negotiates `2026-07-28`, omit `resources.subscribe`: on that revision the flag
means per-resource updates through `subscriptions/listen`, which Maidan does
not implement. `initialize` for every 2025 revision and `2024-11-05` still
sets `resources.subscribe` and still serves `resources/subscribe` and
`resources/unsubscribe`, with the same instructions, on `POST /mcp` and on
`POST /mcp/streamable`. Every result carries `resultType: "complete"`; Maidan
never answers `input_required`.

Every result of the six cacheable operations carries a `ttlMs` and a
`cacheScope` (SEP-2549; `CacheableResult` in the `2026-07-28` schema). The
official TypeScript client caches on them by default and caps a TTL at 24
hours (`MAX_CACHE_TTL_MS`). `public` means a shared gateway may hand the result to
any caller, so Maidan uses it only for a result that is the same bytes whoever
asks. A hint never stands in for authorization: every call is checked against
its token, whatever a client has cached. `POST /mcp/worker` and
`POST /mcp/reviewer` are the exception to the filtered list: each serves one
profile, and a tool the token cannot call is refused when it is called. The
choices live in
`crates/maidan-mcp/src/caching.rs`, and `cache_hints_contract` checks this table
against it.

| Result | `ttlMs` | `cacheScope` | Why |
|--------|---------|--------------|-----|
| `server/discover` | 3600000 | `public` | Versions, capabilities and instructions are the same for every caller and change only with a release. |
| `tools/list` on `/mcp`, `/mcp/streamable` | 3600000 | `private` | Filtered to the token's capabilities, so two tokens get two lists. The catalog changes only with a release, and Maidan cannot announce a deploy (`notifications/tools/list_changed` never fires), so the TTL bounds how long a client keeps a list from before one. |
| `tools/list` on `/mcp/worker`, `/mcp/reviewer` | 3600000 | `public` | A fixed profile, sorted by name, the same bytes for every caller, so a shared cache can keep it (SEP-2567). A tool the token cannot call stays in the list and is refused at `tools/call`. |
| `prompts/list` | 3600000 | `public` | The same for every caller; changes only with a release. |
| `resources/templates/list` | 3600000 | `public` | The same for every caller; changes only with a release. |
| `resources/list` | 3600000 | `private` | Lists the caller's own workspace, which its token fixes. |
| `resources/read` of `maidan://artifacts/{sha256}` | 60000 | `private` | The URI names the bytes, but the read returns the workspace ref (`kind`, `mime_type`, `filename`), which a later upload of the same SHA updates. A minute, the same as a workspace or channel record. Private because access is per workspace. |
| `resources/read` of `maidan://workspaces/{id}`, `maidan://channels/{id}` | 60000 | `private` | Changes on a rename, a topic edit or an archive. A subscriber hears `notifications/resources/updated` and drops its copy at once. |
| `resources/read` of `maidan://boots/{channel_id}` | 60000 | `private` | The workspace boot for a channel. It changes when the glossary or an accepted decision changes, not when a thread is claimed. Private because access is per caller. |
| `resources/read` of `maidan://threads/{id}` | 0 | `private` | Changes with every post, claim and transition. |

---

## Who shows up with which protocol

| They already run | Point them at | Do not |
|------------------|---------------|--------|
| Cursor, Claude Desktop, VS Code, Claude Code, ChatGPT connectors | MCP over `POST /mcp` / Streamable HTTP / stdio, at whichever revision the client requests (`2024-11-05` through `2026-07-28`). Verified against the official TypeScript SDK 2.0 via the MCP Inspector (`2025-11-25`); the others are not yet verified by Maidan's own tests. | — |
| A Python / TS agent they wrote | REST + WS, or MCP if they already have an MCP client. There are thin SDKs for TypeScript, Python, Go and Rust in [`sdk/`](https://github.com/david-engelmann/maidan/tree/main/sdk), at 0.1.0. | An in-process `Crew.kickoff`. Maidan *is* the orchestrator. |
| LangGraph / CrewAI / OpenAI Agents SDK | Recipe on REST+WS (or MCP tools). Those frameworks speak MCP as of 2026; they do not need a Maidan-native runtime. | A LangGraph checkpointer inside Maidan. |
| Another vendor's agent (Salesforce, SAP, Bedrock, Foundry) | A2A Agent Card + JSON-RPC. | IBM ACP. It is A2A now. |
| n8n / Zapier / Make / "we have webhooks" | Outbound webhooks + REST. OpenAPI for the REST half. | A GraphQL gateway. |
| Humans in Slack | The Slack projector (HTTP Events API); link a channel with the `link_slack_channel` MCP tool. Agents stay on MCP/A2A. | Making Slack the datastore. Socket Mode as Marketplace default. |
| Humans in GitHub / GitLab / Gitea | The GitHub projector (App / webhooks); link an issue with `link_github_issue`. Agents use the official GitHub MCP for diffs. | Reimplementing GitHub MCP. Opening PRs as Maidan. Ambient on every PR. |
| Humans in the browser / a React app | `/ui` on this server (the board) and `/ws/subscribe`. | Native AG-UI. CopilotKit is a frontend stack, not a workspace. |
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

**`2024-11-05`-only MCP is not a shippable state, and neither is
`2026-07-28`-only.** The official TypeScript SDK 2.0 requests `2025-11-25` and
rejects a `2026-07-28` answer. A pack or public cut that advertises MCP
while `SUPPORTED_PROTOCOL_VERSIONS = ["2024-11-05"]` will bounce modern
clients. Do not "freeze on 2024" as the strategy. Temporary honesty (J2)
until the upgrade lands is not the same as accepting 2024 forever.

| Protocol | Code today (2026-08-25) | Required | ID |
|----------|-------------------------|----------|-----|
| **MCP** | ✅ **`2026-07-28` shipped** (default; `2025-11-25`, `2025-06-18`, `2025-03-26` and `2024-11-05` accepted since Cluster 412). Stateless Streamable HTTP (no `Mcp-Session-Id`), SEP-2243 `Mcp-Method`/`Mcp-Name` headers, live-wait on `GET /mcp/stream`/WS. | Done–303. | **J3** ✅ |
| **A2A Agent Card** | ✅ Spec v1.0 card, TCK-checked | Spec v1.0 `supportedInterfaces` (`JSONRPC`) | **J4** ✅ |
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

## Gaps worth closing

> The rest of this page is maintainer planning, kept here so the protocol
> decisions and the work they imply stay together. If you are integrating, you
> can stop reading at this line.

J3 shipped (`2026-07-28`). The rest is adapters + honesty. No new native protocol.

| ID | Gap | Size | Notes |
|----|-----|------|-------|
| **J1** | This page | Docs | **Written 2026-08-25.** Keep true when `SUPPORTED_PROTOCOL_VERSIONS` changes. |
| **J2** | ✅ Retired | Docs | Was: "temporary honesty (today 2024-11-05)". No longer needed — J3 shipped; README/Integration now advertise `2026-07-28`. |
| **J3** | ✅ MCP `2026-07-28` **shipped** | Done | Negotiation → stateless streamable core → SEP-2243 routing headers → advertise (default flip + card/reference/Integration). `2024-11-05` still accepted. |
| **J4** | ✅ A2A Agent Card → spec v1.0 `supportedInterfaces` | Done | Keep JSON-RPC URL. Advertise `protocolBinding: JSONRPC`. Do not add gRPC just to fill the array. Signed JWS cards are enterprise-later. |
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
| MCP create-workspace tools so an IDE can bootstrap | Hero seed is REST/CLI by design; the existing catalog already covers it. |
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
5. **Humans already live in Slack** → the Slack projector, not a new protocol.
6. **Humans already live in GitHub/GitLab** → the GitHub projector, not Copilot.
7. **Editor coding agent should work a Maidan thread** → ACP adapter later, not now.

If two of those apply, use two transports. That is the design (README: "one surface, four transports").

---

## See also

- [Integration.md](Integration.md) — start here to actually connect
- [Providers.md](Providers.md) — hosts, not wires
- [Capability Map.md](Capability%20Map.md) — the same ACL on every transport
- [Pre-Public Hardening.md](archive/Pre-Public%20Hardening.md) — section J
- [Expansion Bets.md](archive/Expansion%20Bets.md) — MCP pack, SDK, Slack
- [Path to Impressive.md](archive/Path%20to%20Impressive.md)
- MCP spec `2026-07-28`: https://blog.modelcontextprotocol.io/posts/2026-07-28/
- A2A spec: https://a2a-protocol.org/v1.0.0/specification
- Agent Client Protocol (Zed): https://agentclientprotocol.com/
