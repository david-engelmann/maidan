# Handoff — post-D roadmap pack (2026-08-25)

**You are a coding agent (or human) picking up Maidan work after the
2026-08-25 strategy pass.** This pack is the **strategy and detailed scoping**
behind the post-272 forward work. The **single canonical backlog is
[Open Work.md](Open%20Work.md)** (with [Roadmap.md](Roadmap.md)); the items
below are tracked there. Use this page for the *why* and the detail, then
execute through the normal cluster workflow in [CLAUDE.md](../CLAUDE.md)
(branch → PR → 8 required CI checks → squash/admin-merge → mandatory retro →
`vX.0.0` tag). The IDs here (A–J, S/M/C/E/R, L1–L6) are scoping labels, not a
substitute for opening a cluster.

**Code baseline this pack assumes:** Program D closed at **v266**; clusters
**267–272 all shipped** (tags `v267.0.0`–`v272.0.0` on `main`) — the
optional-deferrals sweep + the LSN read-replica program close. This pack was
drafted 2026-08-25 while 270–272 were still in flight, so some in-body lines
still say "in flight"; the current state is [Open Work.md](Open%20Work.md) /
[CHANGELOG.md](../CHANGELOG.md). J3 (the MCP `2026-07-28` upgrade, `maidan-mcp`)
is the headline open item.

**Star-hold (2026-08-24) still in force until [Launch.md](Launch.md) tag day.**
No GIF/topics/homepage beforehand. Slack and Git are *projectors* (Bet 1 / Bet 6),
not products; both sit after the MCP pack.

---

## Hard rules (fail the session if you break these)

1. **Do not re-do 267–272.** All shipped (tags `v267.0.0`–`v272.0.0`): A2A
   egress content→parts, MCP email tools, workspace import (both modes), search
   token-aware read routing + its metric. Check [CHANGELOG.md](../CHANGELOG.md)
   before starting anything that sounds adjacent.
2. **Do not add a third database engine or a fourth agent protocol.**
   Two SQL dialects (Postgres + SQLite). Industry wires: MCP + A2A + REST/WS.
3. **Do not invent MCP create-workspace tools** so an IDE can bootstrap.
   Seed via REST/CLI. MCP is 78 tools; **today** `2024-11-05`, **J3 required** (`2026-07-28`).
4. **MCP current must be `2026-07-28` (J3 / M.0).** 2024-only is not acceptable.
   Do not ship deeplinks until that upgrade is honest (stateless Streamable
   HTTP, no pretending GET-session is 2026). Pack and public cut wait on J3.
5. **Do not commit** unless asked. Do not clobber or stage
   `compose.override.yaml` (a local dev port-remap override).
6. **Name the claim path `claim_next_thread`** (MCP + REST). EventKind
   wire: `message_posted`, `thread_result_set`, `mention_recorded`.
7. Mail retry is a new `mail_outbox` table, **not** `maidan_outbox`.
8. `/ui` is an operator console, not the product. No SPA, no Playwright
   unless the north star flips.
9. **Do not become Copilot.** Git projector maps issues/PRs to threads and
   posts comments/check runs. Do not clone, commit, or open PRs as Maidan.
   Do not reimplement `github-mcp-server`.

---

## What this pack is (source of truth)

Strategy pack (committed in Cluster 273). The actionable backlog lives in
[Open Work.md](Open%20Work.md); this table maps each pack doc to the slice it
scopes.

| File | Job | When to open it |
|------|-----|-----------------|
| **This page** | Pickup, master ID list, try-out matrix | Always first |
| [Pre-Public Hardening.md](Pre-Public%20Hardening.md) | Polish before public: residue, tests, examples, perf **H**, providers **I**, protocols **J** | Executing A–J |
| [Path to Impressive.md](Path%20to%20Impressive.md) | Strategy: north star, 90-day sequence, why not Slack-clone | Deciding, not implementing |
| [Expansion Bets.md](Expansion%20Bets.md) | Features after 270–272: Slack, Git, MCP pack, SDK, mail | Executing Bet 1–4, 6 |
| [Launch.md](Launch.md) | Production-ready extras, public-preview cut, Show HN | When the question is announce |
| [Providers.md](Providers.md) | Operator host matrix (where it runs) | Recipes, env vars |
| [Protocols.md](Protocols.md) | Operator wire matrix (how it talks) | MCP vs A2A vs REST |
| `docs/README.md`, `docs/Integration.md` | Index + integrator freeze sentence | Linking only |

**Already written (docs-only, treat as done):** Hardening **I1** (Providers.md),
**J1** (Protocols.md). Everything else in the tables below is still open.

Path is strategy; Hardening is polish checkboxes; Expansion Bets is product
slices. If two files mention the same ID, **Hardening owns A–J**, **Expansion
Bets owns S.* / M.* / C.* / E.* / R.*.** Launch owns L1–L6. Path does not own IDs.

---

## Master list — every upgrade / improvement / expansion

Status: **done** = this pack already produced the artifact. **open** = a
later session executes it. **other agent** = do not touch. **parked** =
needs David to un-hold.

### Other agent's ladder (not us)

| ID | What | Status |
|----|------|--------|
| 267 | A2A egress `content → parts` (text-only) | shipped v267 |
| 268 | MCP email-address tools | shipped v268 |
| 269 | workspace import store | shipped v269 |
| 270 | import REST + remap + 409 | shipped v270 |
| 271 | search token-aware replica routing | shipped v271 |
| 272 | search replica-reads counter | **#522** waiting CI (2026-08-25) |

### Hardening A–G — reputation polish

| ID | What | Status |
|----|------|--------|
| A1–A2 | Scrub Cluster/PR diary from public types + impl comments (~771) | open |
| A3 | Vault vs public split policy | open |
| A4 | Wikilink → Markdown on Integration/Production/AGENTS.md | open |
| A5 | CONTRIBUTING/SECURITY "pre-release" tone | open |
| A6 | Fix `mail.rs` module-doc lie (wired, best-effort, no retry) | open (overlaps Bet 4 E.1) |
| B1–B5 | Split monster files (mcp server 2230, pg/sqlite mods, models, Store) | open; not during 270 |
| C1–C4 | Error shape, naming drift, OpenAPI freshness, deprecation policy | open |
| C5 | MCP version honesty until J3, then 2026 copy | **partial** (Integration names 2024 + J3; root README does not) |
| D1–D6 | Evidence.md, ignored-test guide, panic audit, coverage intent, parity, flake | open |
| E1 | `examples/` directory | open (Bet 2 M.1 is the content) |
| E2 | README first screen: docker/binary before `cargo run` | open |
| E3–E5 | Architecture diagram, "what Maidan is not", freeze stale plans | open |
| F1–F4 | Advisory table, release snippet, threat-model vs controls, default-secure demo | open |
| G1–G5 | Root clutter, license headers, issue templates, CODEOWNERS, human changelog | open |

### Hardening H — performance / load (not a product bet)

| ID | What | Status |
|----|------|--------|
| H1 | Postgres + SQLite loadgen baselines checked in | open; parallel with 270 |
| H2 | Agent-shaped mix: MCP / WS / `claim_next_thread` | after H1 |
| H3 | Optional nightly soak (error-rate, not p99) | after H1 |
| H4 | Measured opts only (context filter, search deny-set). No Redis | after H1 numbers |
| H5 | Production.md: stop saying load is "not covered" | open; parallel |
| H6 | Reconcile scale-1.0 gate budgets | later |

### Hardening I — provider hosts (not a third DB)

| ID | What | Status |
|----|------|--------|
| I1 | `docs/Providers.md` | **done** |
| I2 | Ollama/TEI compose + Voyage-as-openai-compatible note | open |
| I3 | R2 / AWS S3 recipes next to MinIO | open |
| I4 | Keycloak + one SaaS OIDC recipe | open |
| I5 | Written Neon/RDS/Supabase: `DATABASE_URL` + pgvector | open |
| I6 | LibSQL/Turso spike (driver flag or no) | spike only |

### Hardening J — integration protocols (not a fourth protocol)

| ID | What | Status |
|----|------|--------|
| J1 | `docs/Protocols.md` | **done** |
| J2 | Holding-pattern copy: today 2024, upgrade required | **partial** |
| J3 | **Required** MCP `2026-07-28` (stateless Streamable HTTP) | **P0**; this is M.0 |
| J4 | A2A Agent Card `supportedInterfaces` (JSON-RPC only) | open |
| J5 | A2A file/data parts on egress | after 267 text |
| J6 | MCP OAuth RFC 8707 only if a real host refuses bearer | spike |
| J7 | n8n/Zapier signed-webhook recipe | open |
| J8 | LangGraph / CrewAI / Agents SDK recipe on REST+WS or MCP | open (with Bet 2/3) |

### Expansion bets (features, after 270–272)

| ID | What | Status |
|----|------|--------|
| **Bet 2 M.0** | **= J3.** Required MCP `2026-07-28`. Not a 2024 freeze | first expansion (P0) |
| M.1 | `examples/` MCP snippets + 10-minute Integration path | after M.0 |
| M.2 | Offline DAG seed (3 scripted agents, no LLM) | after M.1 |
| M.3 | `maidan demo` compose profile | later |
| **Bet 3 C.1** | Freeze ≤15 OpenAPI methods (REST+WS) | after M.0 |
| C.2 | TypeScript client + example bot (`claim_next_thread`) | after C.1 |
| C.3 | Python `maidan` on PyPI | after C.2 |
| **Bet 4 E.1** | `mail_outbox` table + fix `mail.rs` docs | if claiming mail |
| E.2 | SKIP LOCKED worker + backoff + metrics | after E.1 |
| **Bet 1 S.1–S.4** | Slack projector MVP (HTTP Events, mention-only, final message) | after pack/SDK |
| S.5 | Native `chat.startStream` / append / stop | after S.4 |
| S.6 | Slack interactive HITL | after S.5 |
| **Bet 6 R.1–R.4** | GitHub App projector (mention → thread → comment) | after pack; share bridge with Slack |
| R.5 | Check Run queued/in_progress/completed | after R.4 |
| R.6–R.7 | GitLab adapter; Gitea/Forgejo recipe | after GitHub loop is boring |
| Bet 5 | Pointers into Hardening (not a bet) | n/a |
| **L1–L6** | Production-ready extras + public cut ([Launch.md](Launch.md)) | after Hardening P0 + **J3** + M.1; not Slack/Git |
| **K1–K9** | Bug sweep 2026-08-25 (mail lie, Open Work 180, resume panic, outbox SKIP LOCKED, …) | Hardening K; P0 with A6 |
| Star-tax | GIF, logo, OG, topics, homepage | **parked** until Launch tag day |

**Sequence after 270–272:** Hardening P0 + H1/H5 (can overlap) → **J3 / M.0
MCP `2026-07-28` (P0, required)** → M.1–M.2 pack → Bet 3 → Bet 4 if mail
matters → **Launch** (blocked on J3) → Bet 1 *or* Bet 6 → H2–H4 + residue.

If only one expansion: **J3 then Bet 2 pack**. Not Slack first. Not a 2024-only pack.

---

## Try-out matrix — major players and setups

Goal: a new user can try Maidan with whatever they already run. **Ready**
means the *code* path exists. **Recipe** means a page/example is still owed.
**Later** is a bet. **No** is a deliberate non-goal.

### How they run the server

| Setup | Ready? | Next doc / slice |
|-------|--------|------------------|
| `cargo run` + `sqlite::memory:` | Yes (README first command; E2 should demote this) | E2 |
| Docker Compose default / `--profile full` | Yes | Deploy.md |
| Helm / Kubernetes | Yes | Deploy.md |
| Release binary (linux/amd64, linux/arm64, Pi) | Yes | Pi.md, Releases |
| Windows native | No first-class | WSL or Docker; do not add a fourth target |
| Fly / Railway / Render one-click | No | Recipe on Compose/Helm later; not a new runtime |
| Homebrew / nix | No | Star-tax / pack-and-prove leftover |

### Database hosts

| Player | Ready? | Next |
|--------|--------|------|
| Compose Postgres, vanilla PG | Yes | Deploy |
| RDS, Aurora, Cloud SQL, Neon, Supabase, Crunchy, AlloyDB | Same dialect; **recipe owed** | I5 |
| SQLite file / memory / Pi | Yes | Providers, Pi |
| MySQL / Mongo / Dynamo / Cockroach-as-engine | **No** | do not add |
| LibSQL / Turso | Unknown | I6 spike |

### Embeddings / objects / auth / mail / bus

| Player | Ready? | Next |
|--------|--------|------|
| `hash-v1` (default, not semantic) | Yes | warn in prod (E4) |
| OpenAI, Azure OpenAI, Ollama, vLLM, TEI, Voyage-if-`/v1/embeddings` | Yes (one HTTP shape) | I2 recipe |
| Pinecone / Qdrant as primary | **No** | vectors stay with RBAC |
| LocalFs artifacts | Yes | laptop default |
| MinIO, AWS S3, R2, B2, Garage | S3-compatible yes | I3 recipes |
| Native GCS / Azure Blob | **No** unless S3 blocked | |
| OIDC: Keycloak, Authentik, Auth0, Google, Okta | Generic discovery yes | I4 recipes |
| SAML / SCIM | **No** | document OIDC requirement |
| SMTP (SES/SendGrid/Mailgun/Postfix as relay) | Yes, best-effort | Bet 4 for retry |
| Redis / NATS bus | **No** | Postgres LISTEN or in-memory |

### Agent hosts (the "try it from my IDE" list)

| Player | Wire | Ready? | Next |
|--------|------|--------|------|
| Cursor | MCP stdio or Streamable HTTP | Code speaks **2024-11-05 only** — **blocker** | **J3** then M.1. Target **2026-07-28**. |
| Claude Desktop | MCP stdio | Same | M.1 |
| VS Code / Copilot | MCP | Same | M.1 |
| Claude Code | MCP | Same | M.1 |
| ChatGPT connectors / custom GPT | MCP remote | Same; OAuth may block | J6 if they refuse bearer |
| Windsurf, Continue, Cline, Goose, JetBrains | MCP | Same JSON-RPC | M.1 generic snippet covers them |
| Gemini / Vertex | MCP and/or A2A | MCP same; A2A card schema custom | J4 |
| Zed / JetBrains ACP coding agent | Zed ACP | **No native.** Optional worker later | not Bet 2 |
| ChatGPT Assistants / Responses as native wire | — | **No** | they speak MCP now |

### Frameworks and automation

| Player | Wire | Ready? | Next |
|--------|------|--------|------|
| Raw REST + OpenAPI | Yes | `GET /openapi.json` | Bet 3 wraps this |
| WebSocket live events | Yes | Integration.md | |
| LangGraph, CrewAI, OpenAI Agents SDK, PydanticAI | MCP or REST+WS | Code yes; **no recipe** | J8 |
| n8n, Zapier, Make | webhooks + REST | Code yes; **no recipe** | J7 |
| Temporal / Prefect as native | **No** | Maidan *is* the orchestrator | |
| GraphQL / gRPC-for-REST | **No** | OpenAPI is the IT path | |

### Other agents / humans

| Player | Wire | Ready? | Next |
|--------|------|--------|------|
| Another org's A2A agent (Foundry, Bedrock, Salesforce, SAP) | A2A JSON-RPC | Subset yes; card may fail strict SDKs; text-only files | J4, J5 |
| Second Maidan | `/.well-known/maidan.json` | Yes | federation |
| Humans in `/ui` | session + WS | Operator console only | no SPA |
| Humans in Slack | Events API | **No** | Bet 1 projector |
| Humans in GitHub / GitLab / Gitea | App/webhook | **No** | Bet 6 projector. Agents still use official GitHub MCP for repo I/O. |
| Microsoft Teams / Discord | — | **No** | after Slack if ever |
| PagerDuty / Sentry | webhook → thread → `claim_next_thread` | Possible via webhooks | recipe later, not a protocol |
| Copilot coding agent / GitLab Duo | Their runtime | **No** | they live in the forge; we project |
| Grafana / Datadog / Honeycomb | `/metrics` + OTLP smoke | Yes | Production.md |

If a name is not in this table, it is either "speaks MCP or OpenAPI, use those"
or a do-not-chase (IBM ACP, ANP, AP2, A2UI, AG-UI native, stealth Slack).

---

## First slices a later session should actually run

Pick **one**. Default if David says "start":

1. **E2 + C5/J2** — README first screen + holding-pattern MCP copy on the
   *root* README (today 2024, **J3 required**). Half day. Parallel with 270.
2. **A6** — `mail.rs` module docs. Same half day.
3. **H1 + H5** — loadgen Postgres baseline + Production.md honesty.
4. After 270–272 (or a dedicated MCP branch off those files): **J3 / M.0**
   MCP `2026-07-28`, then **M.1 → M.2** pack. That is the try-out story.
   A 2024-only pack is not it.

Do not open Slack or the SDK until J3 is green. **J3 is the first expansion.**

---

## Known nits (do not "fix" by rewriting history)

- `docs/Open Work.md` header may still say an old tag. Other agent's living
  backlog. Leave it unless E5.
- Wikilinks remain in Integration/Production/AGENTS.md (A4).
- CLAUDE.md "latest tag" lags (still mentioned v268 in places). Don't
  fight the other agent on that file except the read-order pointer.
- Expansion Bets residue line may say 765 in Bet 5 vs 771 in the audit
  header. Prefer **771** (2026-08-25 afternoon re-scan).
- Path see-also used to have raw `docs/...` paths; prefer Markdown links.
- Star-hold has no ADR in Decisions.md.

---

## See also

- [Pre-Public Hardening.md](Pre-Public%20Hardening.md)
- [Expansion Bets.md](Expansion%20Bets.md)
- [Path to Impressive.md](Path%20to%20Impressive.md)
- [Providers.md](Providers.md)
- [Protocols.md](Protocols.md)
- [Launch.md](Launch.md)
- [Integration.md](Integration.md)
- `CLAUDE.md` — how to operate in this repo
- `AGENTS.md` — how to connect *to* a running server (not this pack)
