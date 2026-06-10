# Cluster 102.0 — Cross-pod notification fabric

**Theme:** Deliver MCP resource notifications across server replicas, not just within one process.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XIX · tag **`v102.0.0`**.

**Predecessor:** [[Clusters/Cluster 101.0]] (operator gate); MCP notifications from [[Clusters/Cluster 16.0]] / [[Clusters/Cluster 17.0]].

---

## Problem

MCP resource notifications (`notifications/resources/updated`) and streamable sessions fan out through **per-process in-memory state** in `crates/maidan-mcp/src/server.rs` — `subscriptions` (`HashSet`), `pending_notifications` (`Vec`), `notification_tx` (a tokio `broadcast`), and `streamable_sessions`. A tool mutation handled on replica **A** only notifies subscribers connected to **A**; a client subscribed on replica **B** never sees it.

The **event log + bus already cross processes** via Postgres `LISTEN`/`NOTIFY` (`crates/maidan-bus/src/postgres.rs`). This cluster reuses that substrate so resource notifications fan out cluster-wide, making `resources/subscribe` correct behind a load balancer. This is the pattern [[Clusters/Cluster 103.0]] then reuses for presence.

> [!note] Scope boundary
> Notifications fan out cross-pod; **in-flight streamable sessions stay pinned to their pod** (a client keeps its connection to one replica). We make *delivery* correct, not *connection migration*.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Bus** | A cross-process resource-notification channel keyed by resource URI, carried over Postgres `NOTIFY` (single-process / SQLite keeps the in-memory broadcast). |
| **MCP** | `McpServer` subscribes to the shared channel and delivers to its *local* subscribers; resource mutations publish to the channel instead of only the local broadcast. |
| **Server** | Wire the existing resource-fanout points (`tools/call` → thread/channel/workspace/artifact URIs) to the cross-pod publisher. |
| **Tests** | `two_replica_resource_notification_e2e`: subscribe on replica B, mutate on replica A, assert `notifications/resources/updated` arrives on B. |
| **Docs** | [[Architecture]] "cross-pod notifications" section + a [[Decisions]] ADR ("resource notifications ride the event NOTIFY substrate"). |

## Non-goals

- Streamable session migration across pods mid-stream (sessions stay pod-pinned).
- A hard Redis dependency — Postgres `NOTIFY` is the default fabric; Redis stays optional (as for quotas).
- Exactly-once notification delivery — at-most-once is retained; clients reconcile via `resources/read` on (re)connect.

## PR ladder (suggested)

| # | Title |
|---|--------|
| 102.0.1 | `feat(bus): cross-process resource-notification channel (NOTIFY-backed)` |
| 102.0.2 | `feat(mcp): publish/subscribe resource notifications via shared channel` |
| 102.0.3 | `feat(server): route resource fan-out through the cross-pod publisher` |
| 102.0.4 | `test(server): two_replica_resource_notification_e2e` |
| 102.0.5 | `docs(arch): cross-pod notifications + Decisions ADR` |
| 102.0.retro | `docs(retro): Cluster 102.0 + v102.0.0 tag prep` |

## Exit criteria

- A resource update emitted on one replica delivers `notifications/resources/updated` to subscribers on **any** replica.
- Single-process / SQLite behavior is unchanged (in-memory broadcast path retained).
- `two_replica_resource_notification_e2e` green.
- `v102.0.0` tagged after retro.

## Ordering & risks

- **Foundational** for Phase XIX — establishes the cross-pod channel that [[Clusters/Cluster 103.0]] reuses. Do first.
- **Risk — at-most-once NOTIFY:** a dropped notification must be recoverable; document that subscribers reconcile via `resources/read` (mirrors the bus's existing at-most-once posture, see [[Decisions]]).
- **Risk — NOTIFY payload cap (7990 bytes):** notifications carry only a resource URI (small), so the pointer pattern from [[Clusters/Cluster 7.0]] is unnecessary here.

## References

- [[Clusters/Product Ladder 102+]] Phase XIX
- [[Clusters/Cluster 16.0]], [[Clusters/Cluster 17.0]] (MCP notification baseline), [[Clusters/Cluster 7.0]] (NOTIFY pointer pattern)
- [[Architecture]], [[Decisions]], [[Integration]]
