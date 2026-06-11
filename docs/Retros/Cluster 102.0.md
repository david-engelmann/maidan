# Cluster 102.0 retro — Cross-replica MCP resource notifications

> Tag **`v102.0.0`**. First cluster of Product Ladder 102+ (Phase XIX, scale-out core).

## What shipped

- `maidan-bus::ResourceNotifier` — a NOTIFY-backed cross-process channel for MCP resource-update URIs, sibling to the event bus. `InMemoryResourceNotifier` (single process) + `PostgresResourceNotifier` (`LISTEN`/`NOTIFY` on `maidan_resource_updated`). (102.0.1, #275)
- `McpServer` publishes the *unfiltered* URI set to the notifier and runs a per-replica listener loop (`spawn_resource_notify_listener`) that applies the local subscription filter and delivers to SSE subscribers; the inline tool-call response path (`take_pending_notifications`) stays local + synchronous. (102.0.2, #276)
- `maidan-server` wires the notifier per dialect via `AppState::attach_resource_notifier` — Postgres LISTEN/NOTIFY when enabled, in-memory for SQLite / polled-relay mode. (102.0.3, #277)
- `two_replica_resource_notification_e2e`: a mutation on replica B reaches an SSE subscriber on replica A over real Postgres, with a negative case for unsubscribed URIs. (102.0.4, #279)

## What was deferred

- Cross-pod migration of in-flight streamable sessions — sessions stay pod-pinned; only notifications fan out.
- Distributed presence & typing — Cluster **103** (reuses this NOTIFY-fabric pattern).

## Surprises

- Three of the cluster's CI runs failed a required check with the same `collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped` while linking large `maidan-server` test binaries — a runner linker fault, not a code defect (lint + the other jobs passed on the same commit). Fixed mid-cluster with a dedicated CI PR (#278): `RUSTFLAGS=-C debuginfo=line-tables-only` + trimming the `unit tests` job's redundant `--all-targets` build. It also cut CI time (`unit tests` ~4m → ~1m40s).
- `McpServer` already carried an `Option<Arc<dyn EventBus>>`, so the cross-process substrate (Postgres NOTIFY) was proven in-repo before this cluster — the notifier reuses that exact pattern.

## Decisions

- Resource notifications ride a **dedicated** NOTIFY channel with URIs published directly, rather than re-deriving them from domain events on each replica: some fan-outs (`pin_message`, `cast_vote`, reactions) have no 1:1 `Event`. Single delivery path (the originator also delivers via its listener loop), so no de-duplication. See `docs/Decisions.md`.

## Capability table extension

| Capability | Where |
|------------|-------|
| Cross-process MCP resource-update fan-out | `maidan-bus::ResourceNotifier`, `maidan_resource_updated` |
| Per-replica notification delivery | `McpServer::spawn_resource_notify_listener`, `AppState::attach_resource_notifier` |

## Risks

- At-most-once NOTIFY: a dropped resource notification is reconciled by the client re-reading the resource (same posture as the event bus).
- The CI linker flake is mitigated, not proven eliminated; if it recurs, escalate to `debuginfo=0` or a faster linker (lld/mold).

## Next

Cluster **103** — distributed presence & roster.
