# Cluster 79.0 retro — A2A long-running tasks

> Tag **`v79.0.0`**.

## What shipped

- `tasks/cancel` on `POST /a2a/v1/rpc` — persists `TASK_STATE_CANCELED` and returns updated task JSON.
- `SubscribeToTask` / `tasks/resubscribe` emits `statusUpdate` frames while tasks stay non-terminal (polled from store).
- Terminal-task subscribe returns JSON-RPC `-32005`; cancel + progress covered in `a2a_protocol_e2e`.
- [[Agent Integration]] documents cancel params and subscribe progress semantics.

## What was deferred

- Full A2A task marketplace / operator UI.
- Push notification delivery guarantees beyond best-effort HTTP.

## Next

Cluster **80** — delivery ops unified ([[Clusters/Product Ladder 77+]]).
