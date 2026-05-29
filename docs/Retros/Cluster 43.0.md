# Cluster 43.0 retro — UI v2 shell

> Tag **`v43.0.0`**.

## What shipped

- `/ui` v2 layout: responsive grid, channel sidebar, dark live feed panel.
- `GET /ui/api/workspaces/:wid/channels` for session/bearer reads.
- WebSocket live tail in browser (`Connect WS`) with workspace filter + presence when `member_id` set.
- Operator tools (search, FSM, tokens) preserved in collapsible section.

## What was deferred

- Vite/React SPA (still static embedded HTML).
- Thread list in sidebar; create channel/thread in UI (Cluster 44).
- WS auth via session cookie without bearer mint.

## Forward look

Cluster **44**: UI collaboration flows (post/edit, uploads, search UX).
