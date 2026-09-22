# Cluster 404 — Wave 2 #28: member occupancy follows and manager digest

> Post-gate hardening · target tag `v404.0.0` · umbrella issue #942

## Contract

- A member follow is a durable same-workspace subscription. Session callers
  manage only their own follows; bearer orchestrators keep the established
  act-as-any behavior. Self-follow is invalid.
- Occupancy combines ephemeral presence with assigned, non-terminal work.
  Every work item is filtered through the caller's existing thread-access
  rule, so counts cannot reveal private-channel work.
- Meaningful lifecycle events caused by a followed member produce normal
  per-recipient notifications. Existing event-kind, channel, and thread mutes
  remain authoritative, and access is checked again at delivery time.
- The manager digest groups unread notification rows into per-channel
  `result`, `gate`, and `stuck` counts after a caller-supplied watermark. It is
  notification composition, not a performance-analytics projection.
- REST and MCP remain twins, with SQLite/Postgres parity and public contract
  documentation in the same cluster.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 404.1 | #943 | `maidan_member_follows` and dual-backend follow primitives |
| 404.2 | #944 | REST/MCP follow and access-filtered occupancy surfaces |
| 404.3 | #949 | Access-checked, mute-aware followed-member lifecycle routing |
| 404.4 | #947 | Atomic `approval_requested` gate event and routing signal |
| 404.5 | #948 | Notification-derived manager digest over REST, MCP, and email |
| 404.retro | close PR | Ledgers, capability record, and retrospective |

## Non-goals

- Persisting presence in `maidan_events`.
- Team scoring, trends, performance analytics, or a dashboard.
- Changing Cluster 387 run-lineage semantics.
- Cutting a release tag; tags remain a maintainer action.
