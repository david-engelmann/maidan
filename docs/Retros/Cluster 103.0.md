# Cluster 103.0 retro — Distributed presence & roster

> Tag **`v103.0.0`**. Second cluster of Product Ladder 102+ (Phase XIX, scale-out core).

## What shipped

- `maidan-bus::PresenceNotifier` — a NOTIFY-backed channel carrying a typed `PresenceEvent` (Online/Away/Offline/Typing), sibling to the resource notifier. `InMemoryPresenceNotifier` + `PostgresPresenceNotifier` (`maidan_presence`). (103.0.1, #281)
- Distributed `PresenceHub` — publishes local changes; a listener folds remote events into a merged, TTL-expiring roster and fans frames to local subscribers; a heartbeat re-announces local members and a sweep expires stale remotes. `build_snapshot` merges local + non-expired remote members. (103.0.2, #282)
- `maidan-server` wiring — `PostgresPresenceNotifier` attached + `spawn_tasks()` in Postgres NOTIFY mode; single-process (SQLite/polled) keeps the legacy local-only hub. (103.0.3, #283)
- `two_replica_presence_e2e` — a member online on replica A reaches B's live stream **and** roster snapshot, typing crosses, and a disconnect propagates an offline frame. (103.0.4, #284)

## What was deferred

- TTL-expiry-on-crash assertion in the e2e (needs timing); the heartbeat/TTL logic is covered by 103.0.2 unit tests.
- Rich status / DND UX (Slack-grade) — out of ladder scope.

## Surprises

- Planning 103.0.3 surfaced **two real bugs** in 103.0.2, both fixed before #282 merged:
  1. **Heartbeat noise** — the periodic re-announcement was re-firing presence frames to subscribers every interval. Fixed by tagging heartbeats (`PresenceEvent.heartbeat`) and fanning out only on an actual change; heartbeats refresh `last_seen` silently.
  2. **Self-echo regression** — the refactor made a connecting client receive its own "online" frame, breaking `presence_ws_e2e`. Restored the legacy ordering: announce arrival to existing subscribers before the new connection's receiver exists.
- This reinforced the value of running the existing e2e locally before merge.

## Decisions

- Dedicated presence NOTIFY channel with a **typed** `PresenceEvent` (state, not a URI list, so not the resource channel); **receiver-stamped TTL** (skew-safe); single delivery path with `origin`-tagging; **gated to Postgres+NOTIFY** (single-process keeps the legacy hub to avoid pointless heartbeat overhead, unlike the resource notifier which attaches in-memory everywhere). See `docs/Decisions.md`.

## Capability table extension

| Capability | Where |
|------------|-------|
| Cross-replica presence/typing fan-out | `maidan-bus::PresenceNotifier`, `maidan_presence` |
| Merged TTL roster across replicas | `PresenceHub` (heartbeat + sweep), `AppState::attach_presence_notifier` |

## Risks

- At-most-once NOTIFY: a dropped presence delta is reconciled by the next heartbeat (or the TTL sweep).
- Heartbeat/TTL defaults (10s / 30s) are env-tunable (`MAIDAN_PRESENCE_HEARTBEAT_SECS` / `MAIDAN_PRESENCE_TTL_SECS`); too-tight values increase NOTIFY chatter.

## Next

Cluster **104** — durable ephemeral state (app-OAuth codes + reindex jobs).
