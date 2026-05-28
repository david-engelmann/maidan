# Cluster 23.0 retro — Web UI product

> Closing wave for Cluster 23.0 · target tag `v23.0.0` (shipped with ladder PR #198).

Cluster 23.0 made `/ui` usable for core operator flows without raw HTTP.

## What shipped

- **PR #198** (`0cffd8f`) — `/ui` tabs: workspace events, lexical/semantic search,
  thread load + FSM transition + message list, member API token mint (session or bearer).

## What was deferred

| To | What | Why |
|----|------|-----|
| Post-23 | Channel browser, create channel/thread in UI | Vanilla static UI scope. |
| Post-23 | WS live tail in browser | Events tab uses HTTP poll only. |
| Post-23 | React/Vite SPA | Deferred since Cluster H. |
| [[Remaining Work]] | Artifact upload, federation, purge in UI | Ops surfaces stay API-first. |

## Surprises

- Search query param is `q` (not `query`); capability tests caught this in Cluster 22.

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| Operator UI: events, search, FSM, token mint | `v23.0.0` |

## Risks identified + mitigated

- UI drift from API — uses same JSON shapes as HTTP routes.

## Risks identified + still open

- No WS subscription in UI; operators may miss real-time tail without external client.

## Forward look

Shipped with **24–27** in #198; ladder close at **`v27.0.0`**. Next: [[Remaining Work]].

## Acknowledgements

- Maintainer merge #198.
