# Cluster 15.0 retro — MCP `resources/subscribe` notifications

> Closing wave for Cluster 15.0 · target tag `v15.0.0`.

Cluster 15.0 shipped MCP resource subscriptions on stdio with
`notifications/resources/updated`, closing the long-standing subscribe deferral
without taking on streamable HTTP in the same wave.

## What shipped

- **PR #182** — Implementation bundle (15.0.1–15.0.4):
  - `resources/subscribe` + `resources/unsubscribe` methods in `maidan-mcp`.
  - Stdio notification queue with `notifications/resources/updated`.
  - URI validation helper in `resources.rs`.
  - Initial trigger mapping: `tools/call` `post_message` notifies
    `maidan://threads/{id}` subscribers.
  - Tests for subscribe/unsubscribe notification behavior.
  - Architecture + Decisions updates; regenerated MCP reference.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 16  | Streamable HTTP parity for resource notifications | Keep this wave stdio-first and mergeable. |
| Post-15.0   | Broader resource fan-out beyond threads           | Start with deterministic low-risk mapping. |
| Post-15.0   | SQLite semantic (`sqlite-vec`)                    | Separate search epic.                    |

## Surprises

- No transport surgery needed: a dispatcher-level pending-notification queue
  integrated cleanly with existing stdio loop.

## Decisions

- **Stdio-first subscriptions** — ship value for desktop MCP clients now; leave
  HTTP streaming for a dedicated follow-up.
- **Exact URI subscriptions** — no wildcard patterns in this release.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| MCP `resources/subscribe` + `resources/unsubscribe`     | `v15.0.0`          |
| Stdio `notifications/resources/updated`                 | `v15.0.0`          |

## Risks identified + mitigated

- **Polling-only MCP resource clients** — now have push notifications on stdio.

## Risks identified + still open

- **HTTP transport parity** — `POST /mcp` remains request/response only.
- **Fan-out completeness** — only mapped thread updates from `post_message`.

## Forward look

Next: **Cluster 16.0** — MCP streamable HTTP parity for resource notifications.
See [[Clusters/Cluster 16.0]].

## Acknowledgements

Solo cluster. Implementation #182, this retro.
