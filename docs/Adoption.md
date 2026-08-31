> **Reconciled (Cluster 291, 2026-08-27):** David gave the go — the actionable items from
> this pack are folded into [Open Work](Open%20Work.md), the single canonical backlog
> ("Adoption & ecosystem" section). The "new-files-only / do not fold / do not splice into
> Open Work" rules below are **superseded**; this doc now serves as the detailed spec/index
> behind those backlog items. The `sdk/` scaffolds remain gated 0.0.1 name-holds — "do not
> implement the client code without a go" still stands.
# Adoption — from "interesting" to "we run this"

**Audience:** David (and a later agent) scoping the work that sits
*after* someone hears about Maidan. This is not promotion and not
the existing expansion bets restated. It is the gap between a click
and a real user.

**Companion files (do not treat this as replacing them):**
[Promotion.md](Promotion.md) is how we tell people.
[Launch.md](Launch.md) is when we may speak.
[Expansion Bets.md](Expansion%20Bets.md) Bet 3 is a *thin* TS/Python
client. This file is bigger: four language SDKs, a hosted try-it on
maidan.world, and the rest of the decide-to-adopt path.

**Hard rule for this pack:** new files only. Do not fold these IDs
into Open Work / Handoff until David says to. Do not start the
clients or the cloud from this document without a go.

**TypeScript, not Typesense.** "Typesense client" in the ask is read
as **TypeScript**. Maidan already has search. Do not add a Typesense
backend.

Snapshot: 2026-08-26. `main` is **v280.0.0**. Cluster **281**
(benchmark) is in flight on `feat/cluster-281-benchmark`. Star-hold
until public-preview tag day still applies to *announce*; this work
can be built before that.

---

## 0. Progress to take note of (Claude, 273-280)

Do not rebuild these. Adoption work *uses* them.

| Tag | What landed | Adoption meaning |
|-----|-------------|------------------|
| **v273** | Strategy pack folded into Open Work / Roadmap | Handoff is the "why"; Open Work is the backlog |
| **v274** | Launch positioning; A2A labeled experimental | Pitch is operating-layer, not "Slack for agents" |
| **v275** | The pitch (README/docs) | Shareable one-liner exists |
| **v276** | Runtime reports the **tag**, not `0.0.0` | Strangers can trust `/health` |
| **v277** | SQLite defaults to one connection | Quickstart no longer deadlocks under writes |
| **v278** | **One-command two-agent demo** (`compose.quickstart.yaml` + `scripts/quickstart-two-agents.sh`) | Local 5-minute path exists. Still needs Docker. |
| **v279** | **`maidan init`** (first workspace + admin token, refuses if store is used) | Prod bootstrap without `AUTH_DISABLED` |
| **v280** | **LangChain / AutoGen / REST recipes** in `examples/` + `docs/Framework Integrations.md` | Python *recipes*, not a published client |

Still open on Open Work (not this file's job, but they **gate**
adoption): **J3 MCP `2026-07-28`**, A2A v1.0 compliance, published
benchmark (281), GitHub homepage/topics, Bet 3 as originally scoped.

---

## 1. The funnel (what "adopted" means)

```
hear about it          Promotion.md / Show HN / maidan.world
        |
        v
try in 60 seconds      hosted playground (this file, Ad.P)
        |
        v
try in 5 minutes       local quickstart (278; remaining Ad.Q)
        |
        v
write a real agent     language SDK (Ad.K) or MCP pack (J3 + Bet 2)
        |
        v
run for real           self-host (Deploy)  OR  paid/cloud (Ad.Cld)
        |
        v
stay                   docs, versioning, support, packages that
                       do not rot (Ad.D, Ad.H)
```

A star is not adoption. Adoption is: **an agent they wrote is
`claim_next_thread`-ing a Maidan they did not have to invent.**

agent-meet (inspo, 2026-08-26) does the *top* of this funnel
extremely well and almost nothing else:

- Hosted at agentmeet.net. No signup.
- Create a room in the browser, copy an invite prompt, paste it
  into any HTTP-capable agent.
- Three endpoints (join / message / read). They advertise
  "no SDK" because the API is tiny.
- Self-host is an afterthought.

Maidan is the opposite product (durable workspace, many MCP tools,
capability tokens, Postgres). We cannot copy "no SDK" as the
strategy. We can copy the 60-second hosted join. Then we owe
SDKs so the people who stay are not stuck on curl.

Steal from agent-meet:

1. A URL that is live without Docker.
2. An invite prompt that *is* the API docs (the agent does not
   read Integration.md first).
3. A browser window that shows the conversation happening.

Do not steal: polling as the only realtime, a three-endpoint toy
API as the long-term contract, "no SDK" as a badge.

Steal from the 2026 sandbox crowd (Mitos, E2B, Daytona): the
same client talks to hosted and self-hosted. Only base_url plus
token change. If the playground SDK cannot point at
compose.quickstart.yaml, the cloud is a trap.

---

## 2. Master ID list

Prefix **Ad.** so these do not collide with Hardening A, Launch L,
or Git projector R.

| ID | Work | Why it exists | Size |
|----|------|---------------|------|
| **Ad.P** | Hosted playground at maidan.world | 60-second try; agent-meet shaped | Cluster-sized, P0 for conversion |
| **Ad.K** | Language clients: TS, Python, Go, Rust | People will not adopt an 85-tool HTTP surface from curl | 4 packages, TS first |
| **Ad.Q** | Remaining local-path nits | 278 exists; cargo-first README and no invite prompt remain | Small |
| **Ad.I** | Invite prompt + agent card | The actual agent-meet trick | Small once P exists |
| **Ad.D** | Adoption docs on maidan.world | Client reference, cookbooks, playground TOS | Docs, parallel |
| **Ad.H** | OSS hygiene that blocks trust | Package names, issue templates, CoC, compatibility matrix | Small, do early |
| **Ad.Cld** | Real cloud (accounts, isolation, maybe billing) | "I don't want to operate Postgres" | Company-sized. After P. |

J3 (MCP 2026) stays Hardening. Slack/Git stay Expansion Bets. Mail
retry stays Bet 4. None of those are duplicated here.

---

## 3. Ad.P — hosted playground (maidan.world/play)

This is the highest-leverage adoption slice. Without it, Show HN
bounces on "I have to run Docker." With it, a stranger and two
agents share a thread in a minute.

### Product

On maidan.world, a **Create sandbox** button:

1. Mints an isolated workspace (not a shared playground DB that
   everyone can read).
2. Creates two agent members + one human/operator member.
3. Prints:
   - a **viewer URL** (watch the thread in a browser)
   - two **invite prompts** (paste into Cursor / Claude / Codex)
   - an MCP config snippet (after J3: `2026-07-28`)
   - REST `base_url` + scoped tokens
4. TTL: sandbox dies in **4 hours** (configurable). Data is
   deleted. No "free forever workspace."
5. Caps: rate limit (Cluster 183 already exists), message quota,
   artifact bytes, one sandbox per IP/account sliding window.

### What it is not

- Not `AUTH_DISABLED` on the public internet. Ever.
- Not a shared demo workspace other tenants can search.
- Not production Maidan-as-a-service (that is Ad.Cld).
- Not a place we run *their* LLM. They bring the agent; we host
  the workplace.

### Shape that fits this codebase

Reuse the server we have. Do not write a Next.js toy API in front
of it (agent-meet's architecture is Next plus a hosted DB plus
polling — that is their product, not ours).

Recommended v1:

- One (or N pooled) `maidan-server` on Postgres, or an
  ephemeral Fly/Cloud Run instance per sandbox.
- `maidan init`-style provision (279) behind an operator job:
  create workspace, members, capability-scoped tokens, seed a
  hero thread (claim/wait/result).
- Tokens: `message:post` + `workspace:read` + `event:subscribe` +
  `thread:transition` as needed. **Never** `token:admin` on an
  invite.
- Isolation: workspace RBAC we already have (159–165, 180 DMs).
  Confirm search/context cannot leak across playground tenants
  (204 artifact refs, deny-set). Add a `sandbox=true` flag plus
  a sweeper that deletes the workspace at TTL (187/53 erasure).
- Frontend: a page on maidan.world that calls a **playground
  control API** (new, tiny): `POST /play/sandboxes`,
  `GET /play/sandboxes/:id`, `DELETE`. That API is *not* the
  Maidan agent API. Agents still speak MCP/REST/WS to the
  sandbox's Maidan URL.

Pooled-process vs microVM:

| | Pooled Maidan (many workspaces, one server) | Ephemeral instance per sandbox |
|--|---------------------------------------------|--------------------------------|
| Cost | Cheap | Expensive at HN scale |
| Isolation | App-layer RBAC (already built) | Process/network |
| Cold start | Milliseconds | Seconds |
| Blast radius | A bug in RBAC is a cross-tenant leak | A bug is one sandbox |

**v1 = pooled + ruthless caps + erasure sweeper.** Promote to
ephemeral machines only if a leak or noisy-neighbor shows up.
Do not start with E2B/microVMs; we are not a code-execution
sandbox.

### Invite prompt (Ad.I) — steal this from agent-meet

`GET /play/sandboxes/:id/invite.md?role=agent-a` returns Markdown
an agent can be pasted into. It must contain:

- Who they are (name, member id, capabilities)
- Base URL (HTTPS)
- Token (once)
- How to speak: MCP endpoint **or** 6 REST calls, not 85 tools
- The hero loop: `claim_next_thread` then do work then
  `post_message` / `set_thread_result`
- "You are talking to other agents in this workspace. Watch the
  thread; do not dump secrets."

The viewer URL is for humans. The invite is for agents. Mixing
them is how agent-meet's README has to say "don't share just the
room link."

Local 278 should print the **same** invite shape (Ad.Q1) so the
agent a stranger pastes in Cursor works against compose or the
cloud with only `base_url` swapped.

### Hosting

maidan.world already wants to be the product site. Play lives
there (`/play`). The sandbox Maidan process can be
`play-api.maidan.world` (same cert). GitHub Pages cannot run
this; the **site** can stay static, the **playground API** is a
real service (Fly.io, Cloud Run, a small k8s namespace). Budget
for always-on cost and for HN hug-of-death (queue creates, cap
concurrency, shed load with a kind error).

### Slices

| ID | Scope |
|----|--------|
| **Ad.P0** | Threat model for a public sandbox (caps, TTL, no admin tokens, erasure, rate limit, abuse). Write it before code. |
| **Ad.P1** | Control API + sweeper on a dedicated deploy. Seed hero thread. |
| **Ad.P2** | maidan.world `/play` UI: create, copy invite, live viewer (WS or `/ui` constrained to that workspace). |
| **Ad.P3** | Invite.md + Cursor/Claude/Codex snippets. MCP snippet waits on J3; until then REST+invite is honest. |
| **Ad.P4** | Hug-of-death: create-queue, max concurrent sandboxes, static "at capacity" page. |
| **Ad.P5** | TOS + retention + "this is a toy, not your prod data" on the page. |

---

## 4. Ad.Cld — real cloud (later)

"I want a cloud option at maidan.world so people experiment without
spinning up their own" **starts** as Ad.P. It **becomes** Ad.Cld
when someone wants a workspace that lasts longer than a TTL and
holds real secrets.

Ad.Cld is a company:

- Accounts (OIDC — we already speak it)
- Durable tenants (workspaces + quotas 188)
- Backups (260), status page, DPA, abuse desk
- Billing if it is not a charity
- Same SDK `base_url` as playground and self-host
- **Still** not an in-process LLM host

Do not build billing or account dashboards for the public
preview. Do not promise "Maidan Cloud" on Show HN until Ad.P
is real and Ad.Cld has a written tenancy story. The honest
launch sentence:

> Try it at maidan.world/play (sandbox, hours, not prod).
> Run it yourself with compose (278) or Helm. A hosted
> durable cloud is on the map, not shipped.

---

## 5. Ad.K — language clients

Bet 3 was "thin TS then Python, at most 15 methods, REST+WS." Keep
that shape. **Add Go and Rust** because those are the other
two languages people who self-host a Rust server actually
write agents in. Do not generate 200-method OpenAPI monsters.
Do not wrap MCP as the primary transport (MCP is the IDE pack;
the SDK is for bots, Slack projectors, scripts).

### Packages

TypeScript first, then Python, then Go, then Rust.
TypeScript: @maidan/sdk.
Python: maidan.
Go: sdk/go (Go module in this repo).
Rust: maidan-sdk.

The server workspace stays unpublished. These four packages
are published. Register the names now (Ad.H1) even as empty
0.0.0 placeholders so they are not squatted.

Layout in this repo (pin CI to the same tag as the server):

    sdk/typescript/
    sdk/python/
    sdk/go/
    sdk/rust/

One repo until a client needs its own release cadence. Then
split. Do not start with four GitHub orgs.

Name note vs Bet 3: Expansion Bets said @maidan/client. Either
@maidan/sdk or @maidan/client is fine; pick one at Ad.H1 and do not
publish both. Prefer @maidan/sdk so "client" is free for a CLI
later. Python must be maidan (short, obvious) or we lose
the name.

### Frozen surface (all four languages, identical)

Constructor: Client with base_url and token. Token is passed in.
SDKs never mint token:admin.

MVP (at most 15 methods), mapped to existing names:

1. workspaces.create / get — no workspaces.list (still does
   not exist; do not invent it)
2. channels.list / create
3. threads.create / get / transition
4. messages.post / list
5. claim_next_thread
6. subscribe (WS) with helpers:
   wait_for_result, wait_for_mention, wait_for_ready
   (these are MCP live-waits today; the SDK implements them on
   WS message_posted / thread_result_set / mention_recorded
   / thread_ready, not a fake REST long-poll)
7. Artifact presign plus PUT
8. workspaces.import (already shipped in 270)

Language-idiomatic extras, not extra methods: retries on 429
(Retry-After, Cluster 172), typed IDs so a ThreadId cannot
be passed as a ChannelId, error type that carries the Maidan
error shape (Hardening C1).

MCP: optional helper that prints the Cursor snippet. The SDK
does not embed an MCP server.

### Per-language notes

**TypeScript first (Ad.K1).** Unblocks browser /play snippets,
Cursor-adjacent agents, and Bet 1 Slack projector. ESM plus types.
Node 20 and bun. Example: examples/sdk-claim-next.ts that does
the 278 two-agent loop without bash.

**Python second (Ad.K2).** 280 already teaches LangChain/AutoGen
via MCP. The SDK is for people who do not want MCP in-process
(an http bot, a FastAPI worker). Do not make the SDK depend on
the MCP Python package. Keep the 280 recipes; they are a
different door.

**Go third (Ad.K3).** Operators and backend agents. Module path
stable from day one (do not rename). errors.Is for 409/403.

**Rust fourth (Ad.K4).** Ironic to ship last given the server is
Rust, but the users who need a client first are TS/Python.
maidan-sdk must not depend on maidan-server. Types from
maidan-types are tempting — do not publish the whole
types crate as the client. A slim DTO set in maidan-sdk so
we are not locked to server internals. The server workspace
stays unpublished; this crate is the exception (publish true).

### Docs around those clients (Ad.D, with K)

Each language package ships:

- README with the 60-second snippet (constructor, claim-next, post)
- Typed API of the frozen 15 methods (generated from Ad.K0)
- One cookbook: two agents, one thread, wait for result
- A "point this at playground / compose / self-host" note
  (MAIDAN_URL, MAIDAN_TOKEN)

Site pages (maidan.world, after the site exists):

| Page | Content |
|------|---------|
| /docs/clients | Install matrix TS/Python/Go/Rust plus compatibility |
| /docs/clients/typescript | 15-method API |
| /docs/clients/python | same |
| /docs/clients/go | same |
| /docs/clients/rust | same |
| /docs/cookbook/claim-next | The agent work loop |
| /docs/cookbook/two-agents | 278 in SDK form |
| /play "copy as …" | Invite plus TS/Python/Go/Rust snippets |

Do not generate four copies of Integration.md. Integration stays
the protocol bible. Client pages are how to speak it in that
language.

### CI and versioning

- Each SDK release is tagged sdk-ts-0.1.0 *and* notes the
  Maidan server tag it was tested against (v280).
- CI: compose.quickstart up, then SDK smoke (create workspace
  if AUTH_DISABLED-ack, else use a fixture token from
  maidan init), then claim_next, post, subscribe one event.
- A stale SDK is worse than none. If J3 or an OpenAPI break
  ships, bump or yank. Compatibility matrix lives on
  maidan.world/docs/clients (Ad.H6).

### Slices

| ID | Scope |
|----|--------|
| **Ad.H1** | Reserve registry names. Empty README packages. |
| **Ad.K0** | Freeze the 15-method OpenAPI subset in a contract file. Generate types only. |
| **Ad.K1** | @maidan/sdk plus claim-next example plus a 0.x release |
| **Ad.K2** | maidan plus rewrite examples/rest_maidan.py to use it |
| **Ad.K3** | Go module plus example |
| **Ad.K4** | maidan-sdk plus example |
| **Ad.K5** | All four: wait-helpers on WS, 429 retry, ID newtypes |
| **Ad.K6** | Cookbook pages (Ad.D) and playground copy-as snippets |

---

## 6. Ad.Q — local path, what's left after 278

278 is the local story. Remaining nits that still bounce a
stranger:

| Nit | Why |
|-----|-----|
| README still offers cargo run plus sqlite memory as "one line" | HN will try that, hit compile, leave. Hero is compose.quickstart. |
| No invite prompt for the local demo | agent-meet lesson. scripts/quickstart-two-agents.sh should print paste-ready agent text. |
| MCP snippet still 2024-only until J3 | Honest until J3; don't deeplink 2026. |
| No Codespaces / devcontainer | Optional zero-Docker-on-laptop try (Ad.Q2). |
| No brew / cargo-binstall of the release binary | 276 fixed version; a tap is encore. |

Slices: **Ad.Q1** invite text from the quickstart script;
**Ad.Q2** .devcontainer that runs the 278 demo;
**Ad.Q3** README hero = compose (leave cargo as "from source").
Ad.Q3 *is* an existing-file edit (README). Park it until David
lifts the new-files-only rule for this pack, or do it in a
later cluster that is allowed to touch README.

---

## 7. Ad.D — docs that belong on maidan.world

The product site (Promotion: one site, not mdBook-as-homepage)
needs an Adopt section, not just Integrate.

| Page | Content |
|------|---------|
| /play | Sandbox UI (Ad.P2) |
| /docs/quickstart | 278 compose plus invite prompt |
| /docs/clients | Install matrix plus compatibility |
| /docs/clients/{ts,python,go,rust} | 15-method API |
| /docs/cookbook/claim-next | The agent work loop |
| /docs/cookbook/two-agents | 278 in SDK form |
| /docs/connect-cursor | MCP 2026 snippet (after J3) |
| /docs/frameworks | Already: Framework Integrations.md (280) |
| /docs/hosted | Playground TOS, TTL, "not prod"; later Ad.Cld |
| /docs/self-host | Deploy.md / Providers.md as they are |

Until maidan.world exists, keep client READMEs in sdk/* and
this file as the scoping source. Do not splice Adoption into
book/SUMMARY until David says to (same new-files-only rule).

---

## 8. Ad.H — hygiene so it looks like a real OSS project

These are unglamorous and they are exactly what a serious
adopter checks before they depend on you.

H1 reserve registry names (not reserved).
H2 GitHub topics plus homepage maidan.world (empty today).
H3 issue templates for bug, protocol-compat, sdk, playground (missing).
H4 community code of conduct file (missing; CONTRIBUTING and SECURITY exist).
H5 discussions or a start-here pinned issue (none).
H6 compatibility matrix: server tag x SDK tag x MCP rev (none).
H7 security contact actually monitored; security.txt on the site (file exists, site does not).
H8 keep clients MIT, same LICENSE.
H9 changelog per SDK, not only the server CHANGELOG.
H10 support policy: solo maintainer, best-effort, no SLA on playground.

H1 through H4 can happen this week without a cluster. They are
not features. They are the difference between side project and
I will import this.

Also register maidan.world (already the plan).
H2 homepage/topics wait for tag day per star-hold, unless
David wants the homepage field set before announce. The field
itself is not a tweet. H3 and H4 are new files, allowed now.

---

## 9. Other gaps worth naming (not all P0)

Include these so we do not discover them after Show HN. Do not
build them all before launch.

| Gap | Verdict |
|-----|---------|
| Homebrew / cargo binstall of the release binary | Encore (Ad.Q3-ish) |
| Terraform / Pulumi module | Encore; Helm already exists |
| Nix flake | Encore |
| Postman/Bruno collection of the 15 methods | Cheap; do with Ad.K0 |
| Java / C# / Ruby SDKs | No. Four languages is already a lot. |
| Generated full OpenAPI clients | Escape hatch in docs, not the product SDK |
| Discord | Skip until volume. Issues first. |
| Status page | When Ad.P is live |
| DPA / SOC2 | Ad.Cld only |
| workspaces.list | Product hole; if we need it, that is a server cluster with a capability |
| MCP 2026 | J3. Playground MCP snippet waits on it. |
| A2A TCK | Open Work P1. Do not advertise A2A until then. |
| Benchmarks | 281 in flight. Needed for honesty, not for first try. |
| Helm/compose/OpenAPI | Already exist. Point at them; do not rebuild. |

---

## 10. Sequence (what to actually build)

Assume J3 is someone else's cluster. Do not block Ad.P on it
(REST plus invite is enough for v1). Do not block Ad.K1 on Ad.P.

0. Ad.H1 through H4 now — names, homepage, templates, conduct file.
1. Ad.P0 threat model for a public sandbox.
2. Ad.K0 freeze the 15 methods.
3. Ad.P1 through P3 playground plus invite prompt plus /play UI.
   This is the Show HN URL alongside the landing.
4. Ad.K1 TypeScript client. /play copy as TS.
5. Ad.Q1 local quickstart prints the same invite shape.
6. Ad.K2 Python. Point 280's REST example at it.
7. Ad.K3 / Ad.K4 Go, then Rust.
8. Ad.D client plus cookbook pages on maidan.world.
9. Ad.Cld only after P has users and a leak/cost story.

If only one thing: **Ad.P plus invite prompt.** That is what
agent-meet proved. SDKs are how the people who come back stay.
Cloud durable is how you become a company.

---

## 11. Do not

- Do not `AUTH_DISABLED` on maidan.world.
- Do not share one demo workspace across strangers.
- Do not generate a 200-method client from the full OpenAPI.
- Do not put Crew.kickoff in the SDK. Maidan is the runtime.
- Do not mint admin tokens in the client or the invite.
- Do not advertise MCP 2026, A2A v1.0, or Maidan Cloud before
  those sentences are true.
- Do not add a Typesense (or any third) search backend.
- Do not start Slack/Git as an adoption shortcut (Bets 1 and 6).
- Do not fold this list into Open Work until David says so.
- Do not implement J3 from this file. Stay off maidan-mcp
  while another cluster owns it.
- Do not start 270–272 leftovers. Those are closed or owned.
- Do not change any file other than **new** files while this
  pack is being written.

---

## See also

- Promotion.md — telling people; maidan.world as the one site
- Launch.md — when we may speak
- Expansion Bets.md — Bet 3 (thin SDK, overlapping Ad.K but
  TS/Python only)
- Framework Integrations.md — 280 recipes, keep
- agent-meet — hosted join plus invite prompt, not the
  long-term architecture