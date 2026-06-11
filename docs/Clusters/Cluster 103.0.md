# Cluster 103.0 — Distributed presence & roster

**Theme:** Make presence, typing, and the workspace roster consistent across server replicas.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XIX · tag **`v103.0.0`**.

**Predecessor:** [[Clusters/Cluster 102.0]] (cross-pod fabric); presence baseline from [[Clusters/Cluster 99.0]].

---

## Problem

`PresenceHub` (`crates/maidan-server/src/presence.rs`) holds a `HashMap<WorkspaceId, WorkspaceRoom>` behind an `Arc<RwLock<…>>` — **per process**. Presence and typing frames broadcast only to WebSocket clients on the same replica, and `presence_snapshot` reflects only locally-connected members. With >1 replica, every pod has a partial, split-brain view of who is online.

This cluster moves presence/typing onto the cross-pod channel from [[Clusters/Cluster 102.0]] and gives each replica a **merged, TTL-expiring** roster view, so presence is consistent regardless of which pod a member connects to.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Server** | Presence heartbeats + typing events published on the cross-pod channel; each replica maintains a merged view with per-member TTL expiry (server-relative, skew-safe). |
| **WS** | `presence` / `typing` frames fan out cross-pod; `presence_snapshot` reflects the merged roster, not just local connections. |
| **Tests** | `two_replica_presence_e2e`: member online on A appears in snapshot/roster query on B; typing on A seen on B; disconnect on A → TTL expiry observed on B. |
| **Docs** | [[Presence and Roster]] updated with multi-replica semantics (heartbeat interval, TTL, merge rule). |

## Non-goals

- Persistent presence history or "last seen" storage.
- Rich status/DND UX (Slack-grade) — out of scope for this ladder ([[Remaining Work]] §4).
- Exact global ordering of presence transitions — eventually-consistent merged view is sufficient.

## Design (locked)

- **Dedicated channel, not the resource channel.** Presence carries *state*, not a
  URI list, so it gets its own `maidan-bus::PresenceNotifier` (NOTIFY
  `maidan_presence`), a sibling of [[Clusters/Cluster 102.0]]'s `ResourceNotifier`
  — same in-memory/Postgres shape, different payload.
- **Typed `PresenceEvent`** on the wire: `{ origin: replica_uuid, workspace_id,
  member_id, kind: Online | Away | Offline | Typing { thread_id, active } }`.
  `origin` lets a receiver skip its own members in the merged remote view while
  still fanning out to local subscribers (single delivery path → no dedup).
- **Receiver-stamped TTL** (skew-safe): the receiver records `last_seen` with its
  own clock on each event; no sender timestamp is trusted. Heartbeats refresh it.
- **Defaults:** heartbeat **10s** (`MAIDAN_PRESENCE_HEARTBEAT_SECS`), TTL **30s**
  (`MAIDAN_PRESENCE_TTL_SECS`). Explicit `Offline` on disconnect removes promptly;
  absence of heartbeat past TTL expires the member.

## PR ladder

| # | Title |
|---|--------|
| 103.0.1 | `feat(bus): cross-process presence channel (PresenceNotifier + PresenceEvent)` |
| 103.0.2 | `feat(server): distributed PresenceHub (publish + merged TTL view + heartbeat)` |
| 103.0.3 | `feat(server): wire presence notifier in maidan-server` |
| 103.0.4 | `test(server): two_replica_presence_e2e` |
| 103.0.retro | `docs(retro): Cluster 103.0 + v103.0.0 tag prep` |

## Exit criteria

- Presence, typing, and roster are consistent across **≥2 replicas** for a workspace.
- A member's disconnect expires from the roster on every replica within the documented TTL.
- Single-process behavior is unchanged.
- `two_replica_presence_e2e` green.
- `v103.0.0` tagged after retro.

## Ordering & risks

- **After [[Clusters/Cluster 102.0]]** — reuses the cross-pod channel.
- **Risk — heartbeat / TTL tuning:** too aggressive = chatty NOTIFY; too lax = stale roster. Use server-relative TTLs and a documented heartbeat interval; avoid wall-clock comparisons across pods.
- **Risk — thundering herd on reconnect** (e.g. after a rolling update): batch presence snapshots and lean on WS `resume_token` so reconnects don't re-broadcast the whole roster.

## References

- [[Clusters/Product Ladder 102+]] Phase XIX
- [[Clusters/Cluster 102.0]] (fabric), [[Clusters/Cluster 99.0]] (presence v2 baseline)
- [[Presence and Roster]], [[Architecture]]
