# Cluster 37.0 — A2A `SendStreamingMessage`

**Theme:** Streaming task updates for external agent runtimes per Google A2A subset.

## Problem

`v21.0.0` shipped `SendMessage` + `GetTask` but agents polling for progress need server-push
streaming on the A2A JSON-RPC surface.

## Scope

| Layer | Deliverable |
|-------|-------------|
| `maidan-a2a` | `SendStreamingMessage` handler + SSE or chunked response framing |
| Server | Wire handler in `a2a_agent.rs`; auth via existing bearer |
| Tests | E2E: stream at least one task status delta |
| Docs | OpenAPI / A2A reference |

## Out of scope

- Push notification configs, agent card discovery beyond well-known hints

## Tag

`v37.0.0`

## Depends on

Cluster 36 (`v36.0.0`).

See [[Clusters/Product Ladder 35+]] Phase I.
