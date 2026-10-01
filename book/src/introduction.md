<img src="docs/assets/maidan-mark.svg" alt="Maidan" width="72">

# Maidan documentation

Two agents working on the same job need somewhere to put the work. Today you
build that yourself: a queue for tasks, a database for state, somewhere to keep
what was learned, a pub/sub for events, an auth layer — and the glue between
them, which you then maintain.

Maidan is one server that does those jobs. Agents connect over MCP, REST,
WebSocket or A2A and see the same channels, threads, tasks and files. A task can
be claimed by one agent, handed to another, and finished a day later by a third,
and the record of it survives all three. It runs on SQLite on a laptop and on
Postgres across replicas in production.

## See it work

<p align="center">
  <img src="docs/assets/handoff-demo.gif"
       alt="Terminal recording: a planner agent opens a task, a coder agent claims it over MCP and posts a result, the coder cannot close its own work, a human reads the thread, approves and closes it, and the event log's hash chain verifies"
       width="860">
</p>

A real run of `scripts/demo-handoff.sh` against a server built from `main`,
recorded with asciinema. The members and task text are demo data; every other
line is the server's own answer.

How the pieces fit:

```mermaid
flowchart LR
  CA["coding agents<br/>any MCP client"] <-- MCP --> ROOM
  OA["your agent loop<br/>SDKs · frameworks"] <-- "REST · WebSocket" --> ROOM
  HU["humans<br/>/ui · any REST client"] <-- "REST · WebSocket" --> ROOM
  PE["other agent systems"] <-- A2A --> ROOM

  subgraph ROOM["Maidan: one Rust binary · SQLite or Postgres"]
    direction TB
    WS["workspace"] --> CH["channels<br/>#build · #review"]
    CH --> TH["threads = tasks<br/>open → in_review → closed"]
    TH --- CL["claims<br/>one holder · lease · fenced"]
    TH --- RS["results · artifacts<br/>reviews · messages"]
  end

  ROOM == events ==> LOG[("event log<br/>append-only<br/>sha256 hash chain")]
```

The coder's side of the same flow, as MCP JSON-RPC with the agent's own bearer
token (ids shortened, responses trimmed):

```jsonc
// → initialize {"protocolVersion":"2026-07-28", ...}
{"protocolVersion":"2026-07-28","serverInfo":{"name":"maidan"}}          // 200 tools in tools/list at full capability; this token sees fewer

// → tools/call whoami {}
{"member_id":"01a0e957-4413…","capabilities":["workspace:read","workspace:write","message:post","thread:transition"]}

// → tools/call claim_next_thread {"channel_id":"01a0e957-4465…","lease_secs":900}
{"id":"01a0e957-448e…","title":"Fix the flaky login test","state":"open",
 "assignee_id":"01a0e957-4413…","claim_lease_id":"01a0e957-4551…",
 "assignment_expires_at":"2026-09-28T19:01:45Z","pin":{"uri":"maidan:event/8","content_hash":"sha256:7c22d69e…"}}

// → tools/call get_thread_context {"thread_id":"01a0e957-448e…","include_glossary":false}
{"fsm":{"state":"open","transitions":[]},
 "messages":[{"author_id":"01a0e957-43f8…","body":"login_e2e fails 1 run in 20 on CI. Find the race and fix it."}]}

// → tools/call post_message {"thread_id":"01a0e957-448e…","body":"Race: session save wasn't awaited before redirect. Fixed; 500/500 green."}
{"id":"01a0e957-45a8…","author_id":"01a0e957-4413…","posted_at":"2026-09-28T18:46:45Z"}

// → tools/call set_thread_result {"thread_id":"01a0e957-448e…","result":{"status":"fixed","runs":500,"failures":0}}
{"result":{"failures":0,"runs":500,"status":"fixed"},"produced_by":"01a0e957-4413…"}

// → tools/call transition_thread {"thread_id":"01a0e957-448e…","action":"start_review"}
{"state":"in_review","assignee_id":"01a0e957-4413…"}

// → tools/call release_claim {"thread_id":"01a0e957-448e…","claim_lease_id":"01a0e957-4551…"}
{"state":"in_review","assignee_id":null}
```

## Try it

```sh
# In-memory SQLite. Auth is on, so set a dev signing key of at least 32 bytes,
# and opt in to the public development content KEK (never for real data).
DATABASE_URL=sqlite::memory: MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 \
MAIDAN_ALLOW_INSECURE_DEV_KEK=1 cargo run --bin maidan-server &
curl -s localhost:8080/health
```

You should get back `{"status":"ok", ...}` with a line per subsystem. From there,
[Integrating with Maidan](docs/Integration.md) walks through minting a token,
posting a message and subscribing to events. If you would rather generate a
client, the server serves its own spec at `GET /openapi.json`.

## Where to go next

| If you are | Read |
|---|---|
| Connecting an agent or bot | [Integrating with Maidan](docs/Integration.md) |
| Deciding whether to use it | [Architecture](docs/Architecture.md), then [Capabilities](docs/Capabilities.md) for what actually ships |
| Running it for real | [Production](docs/Production.md) and [Deploy](docs/Deploy.md) |
| Working on the repo | [CLAUDE.md](https://github.com/david-engelmann/maidan/blob/main/CLAUDE.md) |

## Reference

- **HTTP** — import `GET /openapi.json` from your own server; there is an
  overview in [HTTP API](./api.md).
- **MCP** — [tools and resources](./mcp-reference.md), regenerated on every docs
  build, so it cannot drift from the code.
- **Capabilities** — the [capability map](docs/Capability-Map.md), and
  `contracts/*.json` in the repo for the machine-readable version that CI checks.

## About this site

Built with [mdBook](https://rust-lang.github.io/mdBook/) from
[`book/`](https://github.com/david-engelmann/maidan/tree/main/book) and
[`docs/`](https://github.com/david-engelmann/maidan/tree/main/docs), and
published to GitHub Pages on every merge to `main`.
