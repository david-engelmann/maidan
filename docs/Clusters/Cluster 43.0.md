# Cluster 43.0 — UI v2 shell

**Theme:** Channel list, WS live event tail, responsive layout (enhanced `/ui`).

## Problem

The operator UI polled events over HTTP only and had no channel browser or real-time tail.

## Scope

| Layer | Deliverable |
|-------|-------------|
| UI | Responsive shell: sidebar channel list + live WS feed |
| API | `GET /ui/api/workspaces/:wid/channels` (session or bearer) |
| WS | Browser subscribe with bearer + optional `member_id` |

## Tag

`v43.0.0`

See [[Clusters/Product Ladder 35+]] Phase III.
