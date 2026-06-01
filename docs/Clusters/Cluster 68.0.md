# Cluster 68.0 — Automation delivery guarantees

**Theme:** Durable, retried delivery for signed HTTP automations (webhooks, slash commands, FSM hooks).

## Problem

Clusters **50–52** invoke external `http` handlers with HMAC signing and a short timeout, but failures are **fire-and-forget** (log + drop). Long-running agent supervisors need **at-least-once delivery attempts** with operator visibility — the same reliability bar as the transactional outbox for bus publish.

Slash and FSM hooks already support `handler_kind: http` ([[Clusters/Cluster 51.0]], [[Clusters/Cluster 52.0]]). This cluster does **not** add a new handler type.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_automation_deliveries` (or extend `maidan_webhook_deliveries` pattern): payload hash, target URL, attempt count, next_retry_at, dead_letter_at |
| Worker | Background dispatcher shared by webhook, slash, and FSM HTTP paths (or thin adapter per source) |
| HTTP | `GET /workspaces/:wid/automation/deliveries` (filter: pending / dead_letter); `POST .../deliveries/:id/replay` |
| Metrics | Counters/histograms: dispatch latency, DLQ depth (feeds Cluster **76**) |
| Docs | [[Production]] runbook: retry policy, max attempts, signing headers |
| Tests | e2e: failing endpoint → retries → DLQ → replay succeeds |

## Non-goals

- New MCP tools for automation registration (existing tools stay).
- Inline dispatch on `transition_thread` only (bus path remains source of truth).
- Exactly-once guarantee to external URLs (integrators must idempotent-handle).

## PR ladder (suggested)

| # | Title |
|---|--------|
| 68.0.1 | `feat(store): automation delivery schema + sqlite/postgres` |
| 68.0.2 | `feat(server): automation delivery worker + metrics` |
| 68.0.3 | `refactor(server): route webhook/slash/fsm HTTP through dispatcher` |
| 68.0.4 | `feat(server): list/replay automation deliveries HTTP` |
| 68.0.5 | `test(server): automation delivery e2e` |
| 68.0.retro | `docs(retro): Cluster 68.0 + v68.0.0 tag prep` |

## Exit criteria

- Failed HTTP automations land in a queryable DLQ within the workspace.
- Operator can replay a dead-letter delivery and observe success in audit log.
- `v68.0.0` tagged after retro.

## References

- [[Clusters/Product Ladder 68+]] Phase XI
- [[Retros/Cluster 52.0]] deferred items
- [[Clusters/Cluster 50.0]] webhook delivery baseline
