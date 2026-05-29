# Cluster 44.0 retro — UI collaboration flows

> Tag **`v44.0.0`**.

## What shipped

- `/ui` v3: thread list per channel, collab panel (post/edit, artifact upload).
- Session/bearer read proxies: threads, messages, faceted search under `/ui/api/...`.
- Faceted search UI: channel, author, kind filters wired to `SearchQuery`.
- Writes require bearer token (mint from session or Tokens tab).

## What was deferred

- React/Vite SPA; inline edit in message list only via details panel.
- Session-cookie auth for POST/PATCH without bearer mint.
- Rich artifact preview in UI.

## Forward look

Cluster **45**: next Phase III operator-product item per [[Clusters/Product Ladder 35+]].
