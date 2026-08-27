> **Reconciled (Cluster 291, 2026-08-27):** David gave the go — the actionable items from
> this pack are folded into [Open Work](Open%20Work.md), the single canonical backlog
> ("Adoption & ecosystem" section). The "new-files-only / do not fold / do not splice into
> Open Work" rules below are **superseded**; this doc now serves as the detailed spec/index
> behind those backlog items. The `sdk/` scaffolds remain gated 0.0.1 name-holds — "do not
> implement the client code without a go" still stands.
# Clients — incorporating Maidan into existing workflows

**Audience:** whoever implements the Python / TypeScript / Rust
packages, and whoever later wires those packages (or MCP / A2A)
into an app that already exists.

**Companions:**
- [Client Contract.md](Client%20Contract.md) — frozen method list and HTTP map
- [Client Testing.md](Client%20Testing.md) — shared scenarios + CI; this is also extra coverage of the server
- [Adoption.md](Adoption.md) Ad.K — funnel view (playground, Go, cloud)
- [Protocols.md](Protocols.md) — which wire to pick; do not contradict it
- [Framework Integrations.md](Framework%20Integrations.md) — LangChain / AutoGen / REST recipes already on `main`

**New-files-only.** Do not splice this into Open Work, Handoff,
README, or book/SUMMARY until David says to. Do not start the
implementation from this document without a go.

Snapshot: 2026-08-26. Server `main` is **v280.0.0**. The three
registry names are reserved as **0.0.1 name holds**. Cluster 281
(benchmark) is someone else's branch.

---

## 0. What "done" means

Working through this pack should leave a stranger able to:

1. Drop `maidan` 0.1 into a Python, TypeScript, or Rust agent they
   already wrote, and run the 278 two-agent loop (claim, post, wait
   for a result) against compose.quickstart or any self-host.
2. Point LangChain / AutoGen / Cursor at Maidan over **MCP** using
   the recipes that already live in `examples/` (the SDK does not
   reimplement MCP).
3. Point another vendor's agent at Maidan over **A2A** using the
   honest recipe in Protocols.md (production *subset*, custom Agent
   Card, text-only egress). Not a fourth client library.
4. Hook n8n / Zapier on **webhooks + OpenAPI** without an SDK.

The packages are how people *stay*. MCP / A2A / webhooks are how
people who already have a stack *arrive*. Both have to work.

Go is `sdk/go` in this repo (no central registry). Out of this pack.

---

## 1. Which door (do not pick a winner)

These are **layers**, not alternatives. Protocols.md is the source.
The SDK is one door. Using MCP or A2A when that is what they already
speak is the efficient path, not a cop-out.

| They already have | Door | What we ship | What we do not |
|-------------------|------|--------------|----------------|
| A Python / TS / Rust agent they wrote | **REST + WebSocket SDK** (`maidan` 0.1) | Typed speaker for the frozen contract | An in-process `Crew.kickoff`. Wrapping MCP as the SDK transport |
| LangChain, AutoGen, Cursor, Claude Desktop, a 2026 MCP host | **MCP** | Keep `examples/langchain_maidan.py` and `autogen_maidan.py` green. SDK may expose `client.mcp_url` (a string) so a README can print the snippet | A Python extra that vendors `mcp`. Pretending the server speaks `2026-07-28` before J3 |
| Another org's agent (Salesforce, SAP, Bedrock, Foundry) | **A2A** JSON-RPC + Agent Card | A cookbook that POSTs `SendMessage` at `/a2a/v1/rpc` and says the card is custom until J4 | An A2A SDK wrapper in 0.1. gRPC. IBM ACP |
| n8n / Zapier / Make | **Webhooks + OpenAPI** | Point at `GET /openapi.json` and `/workspaces/{wid}/webhooks`. J7 is docs | A GraphQL gateway. SDK methods for webhooks in v1 |
| Humans in Slack / Git | Bet 1 / Bet 6 projector | Slack adapter **consumes** the TS SDK later | Making Slack the datastore |

If two apply, use two doors. That is the design.

**Why the SDK is still REST + WS.** A Slack adapter, a cron worker, and
a FastAPI service should not need an MCP host. Bet 3 said this. MCP
live-wait tools (`wait_for_result` and friends) already exist; the
SDK wait helpers wrap **WebSocket** `subscribe` so the same names
work without MCP. The MCP tools stay the framework door.

**Why we still test MCP and A2A.** The shared scenarios in
[Client Testing.md](Client%20Testing.md) re-run the hero loop over
MCP (and one A2A smoke) against the **same-commit** server. That is
how client work covers the main app's other transports, without
making those transports the SDK.

J3 (MCP `2026-07-28`) is required for a public MCP pack. It is **not**
required to prove SDK 0.1. Until J3, MCP recipes say `2024-11-05`.

---

## 2. Repo layout — `maidan/sdk`, independent versions

**Locked 2026-08-26.** Python, TypeScript, and Rust live in this
repo under `sdk/`. Do not create per-language repos. Do not create
a `maidan-clients` repo. The three languages stay together so the
contract cannot drift.

The firewall is **versioning**, not a second GitHub repo.

- Server keeps `v280.0.0` (then v281, …).
- Clients keep their own SemVer. `0.0.1` is the name hold. First
  usable is `0.1.0`. A 0.1.x names the **server tag it was tested
  against** (start at v280); that is a compatibility pin, not a
  lockstep version.
- A server tag is **not** a client release. Publish a client only
  on an explicit tag (`sdk-python-0.1.0`, or `sdk-0.1.0` when all
  three bump together).
- Path filters: a `crates/` PR does not publish PyPI, the JS
  registry, or crates.io. It **does** run the client black-box job
  when it touches claim-next / MCP / events / A2A. That is the
  coverage bargain in [Client Testing.md](Client%20Testing.md).
- Rust `sdk/rust` is **not** a member of the server workspace
  (`publish = false` there). Its own Cargo.toml, `publish = true`.
  Do not path-depend `maidan-server` or publish `maidan-types`.

```
maidan/
  crates/                 # unpublished server workspace
  contracts/              # CI-enforced HTTP / MCP / WS maps
  examples/               # recipes the clients must keep working
  compose.quickstart.yaml
  sdk/
    python/               # PyPI `maidan`
    typescript/           # JS-registry `maidan`
    rust/                 # crates.io `maidan`
    go/                   # module github.com/david-engelmann/maidan/sdk/go
    README.md
  docs/Clients.md
  docs/Client Contract.md
  docs/Client Testing.md
```

Why this uses the maidan repo well:

- **Same-commit server.** Client tests hit the binary built from
  *this* SHA. Extra black-box coverage of claim-next, capabilities,
  WS, MCP, and A2A — not a duplicate of store unit tests.
- **One compose.** The existing `e2e` job already builds
  `maidan-server:dev`. Client CI is a job against that stack.
- **Contracts as source of truth.** The SDK reads
  `http-capability-map.json`, `mcp-tool-names.json`,
  `event-kinds.json`, `ws-subscribe-filter.schema.json`. It does
  not grow a private spec.
- **OpenAPI from the running server.** `GET /openapi.json` on the
  compose image is the freeze check. Optional later: a utoipa tag
  `sdk-v1` that generates **types only**.
- **Examples stay recipes.** `examples/rest_maidan.py` lifts into
  the Python package. LangChain / AutoGen stay MCP recipes and
  become CI canaries.

A stale client is worse than none. If wire names move, bump the
**client** package (or yank it). Do not wait for the next server
tag, and do not publish clients because a server tag happened.

### Split is not the plan

Independent tags are the answer to "I do not want every maidan
change to release the clients." Split a language out only if that
fails in practice (workspace `publish = false` actually blocks
crates.io, or path filters keep misfiring). Not scheduled. Not at
0.1. If it ever happens, pin `MAIDAN_SERVER_TAG`, clone this repo
at that tag for compose, do not mirror a second OpenAPI.

## 3. What to build (shared)

Implement the frozen surface in
[Client Contract.md](Client%20Contract.md) in all three languages.
REST plus WebSocket. Token is passed in; the client never mints
`token:admin`.

Shared extras, not extra methods:

- 429 retries honoring `Retry-After` (Cluster 172)
- Typed IDs so a thread id cannot be passed as a channel id
- Errors that carry the Maidan HTTP error shape
- Env `MAIDAN_URL` and `MAIDAN_TOKEN` as the default constructor
  inputs (explicit args still win)
- `client.mcp_url` → `{base_url}/mcp/streamable` (a string, no MCP
  client dependency). README uses it for the LangChain snippet.
- Ignore unknown JSON fields and unknown WS `kind` strings

Hero loop the README must show:

    claim_next_thread -> do work -> messages.post
    and/or threads.set_result -> wait_for_result

That loop is also scenario H in [Client Testing.md](Client%20Testing.md).
It must pass over REST+WS in every language **and** over MCP in at
least Python (the existing recipes), against the same compose.

First usable release is **0.1.0**. Leave 0.0.1 as the name hold.

---

## 4. Work order

Do not start without a go. When started, this order:

0. **Ad.K0** — freeze [Client Contract.md](Client%20Contract.md)
   against `GET /openapi.json` on a v280 (or current `main`)
   server. If OpenAPI and the contract disagree, OpenAPI plus
   `contracts/http-capability-map.json` win; patch the contract.
   Confirm MCP twins in `contracts/mcp-tool-names.json` still match
   the hero names (`claim_next_thread`, `post_message`,
   `wait_for_result`, `wait_for_mention`, `wait_for_ready`,
   `renew_claim`, `get_thread_context`, `list_messages`).
1. **Python 0.1** first. 280 already has `examples/rest_maidan.py`
   (context + post). Lift that into `sdk/python`, add claim-next
   and WS waits. Point the example at the package. Keep
   `examples/langchain_maidan.py` / `autogen_maidan.py` as MCP
   recipes; do not make the package depend on `mcp`.
2. **TypeScript 0.1.** ESM + types. Node 20 and bun. Must run in
   a browser too (playground "copy as TS" later). `fetch` +
   `WebSocket`. Zero deps if those are global.
3. **Rust 0.1.** reqwest + tokio-tungstenite. Must not depend on
   `maidan-server`. Slim DTOs in this crate. `publish = true`.
4. **Tests + CI** — follow [Client Testing.md](Client%20Testing.md).
   compose.quickstart (reuse the image the `e2e` job already
   builds), fixture token or AUTH_DISABLED, then the shared
   scenario list in every language. Fail the release if it does
   not pass. MCP hero-loop canary in Python. A2A smoke is
   skip-on-card-mismatch until J4, not silent-pass.
5. **Cookbooks** — one REST+WS cookbook per language that *is* the
   278 loop. One MCP paragraph that points at the existing
   examples. One A2A paragraph that points at Protocols.md and
   does not oversell.

---

## 5. Per language (when you sit down)

**Python (`sdk/python`, package `maidan`).** httpx (async client
is fine; offer a sync wrapper if cheap). websockets or httpx's
WS. Hatchling, src layout already there. Classifier moves from
Planning to Alpha at 0.1.0. Rewrite `examples/rest_maidan.py`
to import `maidan`. Test runner: pytest against compose.

**TypeScript (`sdk/typescript`, package `maidan`).** Unscoped
name, already reserved. ESM, `"types"` pointing at generated or
hand-written `.d.ts`. Do not pull node-fetch. Browser-safe (no
`node:` builtins in the public path). Test runner: node:test or
vitest against compose.

**Rust (`sdk/rust`, package `maidan`).** Edition 2021, tokio
optional via features if you can keep the sync surface small;
async-first is fine. `publish = true`. Do not path-dep the
server crates. Tests live in *this* crate's `tests/` so
`cargo test -p maidan` (the client) is not `cargo test -p
maidan-server`.

Each README: 60-second snippet, capability list the hero loop
needs (`message:post`, `workspace:read`, `event:subscribe`,
`thread:transition`), "this talks to hosted, compose, and
self-host; only base_url plus token change," and a pointer at
the MCP / A2A doors for people who should not use the SDK.

---

## 6. Do not

- Do not generate a 200-method client from the full OpenAPI.
  Types-only from a curated `sdk-v1` utoipa tag is allowed.
- Do not wrap MCP as the primary SDK transport. MCP is the
  framework / IDE door. The SDK may print the URL.
- Do not mint admin tokens in the client.
- Do not add `workspaces.list` (the server still has no such
  route).
- Do not put `Crew.kickoff` in the SDK. Maidan is the runtime.
- Do not depend on J3, the playground, or a cloud to prove 0.1.
  Local compose.quickstart is enough.
- Do not invent a fourth protocol.
- Do not add create-workspace MCP tools so an IDE can bootstrap.
  Hero seed is REST/CLI by design.
- Do not fold this into Open Work until David says so.
- Do not implement from this file without a go.
- Do not splice Open Work, Handoff, README, or book/SUMMARY.

A stale client is worse than none. If wire names move, bump the
package or yank the release.

---

## See also

- [Client Contract.md](Client%20Contract.md) — method to HTTP map, MCP twins
- [Client Testing.md](Client%20Testing.md) — scenarios, CI, server coverage
- [Integration.md](Integration.md) — protocol bible (do not copy)
- [Protocols.md](Protocols.md) — layers, not winners
- [Framework Integrations.md](Framework%20Integrations.md) — LangChain / AutoGen / REST
- [Adoption.md](Adoption.md) — funnel, playground, Go, cloud
- [Expansion Bets.md](Expansion%20Bets.md) Bet 3 — historical SDK bet; this pack is the work order
- `contracts/http-capability-map.json` — CI-enforced HTTP map
- `contracts/mcp-tool-names.json` / `mcp-capability-map.json`
- `contracts/event-kinds.json` — WS `kind` strings
- `contracts/ws-subscribe-filter.schema.json` — subscribe frame
- `sdk/README.md` — folder layout for implementers