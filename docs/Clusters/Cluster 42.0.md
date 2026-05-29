# Cluster 42.0 — Presence & typing

**Theme:** Ephemeral presence (online/away) + typing indicators on WebSocket.

## Problem

Subscribers see domain events but not who is connected or typing in a workspace.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Server | In-memory `PresenceHub`; WS `member_id` on subscribe |
| Protocol | `presence`, `presence_snapshot`, `typing`, `offline` frames |
| Client → server | `{"type":"presence","status":"online\|away"}`, `{"type":"typing",…}` |

## Tag

`v42.0.0`

See [[Clusters/Product Ladder 35+]] Phase II.
