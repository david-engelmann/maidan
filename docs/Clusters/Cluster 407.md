# Cluster 407 — Wave 4 #39: executable surface and hero-loop contracts

> Post-gate hardening · target tag `v407.0.0` · umbrella issue #963

## Contract

- Derive the `/ui` HTTP path census from the embedded JavaScript, the
  session-proxy routes from the Axum router, and public operations from
  OpenAPI. A stale path or proxy method mismatch must fail without a browser.
- Give every `EventKind` an explicit REST/MCP production disposition backed by
  executable evidence; intentional internal-only and single-surface events are
  classifications, not parity failures.
- Exercise the human collaboration loop through a real signed session and the
  `/ui/api` boundary, including the corresponding live WebSocket event.
- Check deterministic golden shapes for portable workspace exports and event
  snapshot/catch-up frames while normalizing IDs, timestamps, and signatures
  that are not wire semantics.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 407.1 | #965 | `/ui` fetch templates ↔ OpenAPI/session-proxy contract |
| 407.2 | #966 | Exhaustive `EventKind` × REST/MCP disposition and artifact parity repair |
| 407.3 | pending | Signed-session `/ui` hero loop with live WS observation |
| 407.4 | pending | Normalized workspace-export and event-frame golden fixtures |
| 407.close | close record | Ledgers, executable evidence, and retrospective |

## Non-goals

- A SPA rewrite, a new browser framework, or screenshot/layout release gates.
- Artificial REST/MCP writers for internal worker events.
- A route manifest that competes with OpenAPI, or an event schema that
  competes with `EventKind`.
- Freezing random identifiers, timestamps, signatures, or storage ordering as
  compatibility promises.
