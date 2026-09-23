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

## Try it

```sh
# In-memory SQLite. Auth is on, so set a dev signing key of at least 32 bytes.
DATABASE_URL=sqlite::memory: MAIDAN_SESSION_SECRET=dev-session-secret-change-me-0123456789 \
  cargo run --bin maidan-server &
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
