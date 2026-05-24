# Cluster H retro — Web UI, MCP stdio, production polish

> Closing wave for Cluster H · target tag `v0.7.0`.

Cluster G federated deployments. Cluster H makes Maidan usable from
desktop MCP clients and a browser: stdio transport, SSE event stream,
minimal `/ui`, graceful shutdown, and split health probes.

## What shipped

- **PR #92** — `feat(cluster-h): web UI, MCP stdio, SSE stream, and production polish` (H.1–H.4)

## What was deferred

| To           | What                                              | Why                                      |
|--------------|---------------------------------------------------|------------------------------------------|
| Cluster 1.0  | Full production runbook expansion                   | `docs/Production.md` started in H.       |
| Track W      | mdBook / docs site                                  | Cross-cutting.                           |
| Post-1.0     | Faceted search UI                                   | Needs richer API filters.                |
| Post-1.0     | Full MCP streamable HTTP spec                       | Maidan SSE subset ships in v0.7.0.       |
| Cluster A    | Helm chart                                          | Still deferred.                          |

## Surprises

- **Stdio + async MCP** — blocking stdin loop runs on a current-thread
  runtime inside `spawn_blocking`.
- **Health split landed early** — `/health/live` and `/health/ready` ship
  in H though 1.0 also planned probe semantics.

## Decisions

- **Vanilla `/ui`** — single `index.html` with localStorage token; no frontend build.
- **`GET /mcp/stream`** — SSE mirrors WebSocket JSON event payloads; not full MCP streamable HTTP.
- **`MAIDAN_ENV=production`** — rejects `AUTH_DISABLED` at config load.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Graceful shutdown on SIGINT                             | `v0.7.0`           |
| `X-Request-Id` on HTTP responses                        | `v0.7.0`           |
| `/health/live` + `/health/ready`                        | `v0.7.0`           |
| `maidan mcp-stdio`                                      | `v0.7.0`           |
| `GET /mcp/stream` (SSE)                                 | `v0.7.0`           |
| Browser UI at `/ui/`                                    | `v0.7.0`           |

## Risks identified + mitigated

- **UI token in localStorage** — acceptable for v0.7.0 operator tool; not for shared machines.

## Risks identified + still open

- **SSE backpressure** — bounded mpsc; slow clients dropped silently.
- **mcp-stdio SQLite only** — Postgres stdio deferred.

## Forward look

Cluster 1.0 locks semver and production gates. Cut `v0.7.0` after this retro merges.

## Acknowledgements

Solo cluster. H.1–H.4 delivered in one PR.
