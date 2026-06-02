# Cluster 72.0 — A2A task streaming

**Theme:** Persisted push notification config and `SubscribeToTask` SSE for non-terminal tasks.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_a2a_push_configs`, `maidan_a2a_tasks` (migrations 0027 / 0026) |
| A2A RPC | `SubscribeToTask` + `tasks/resubscribe` alias; push config via store |
| Push | Best-effort HTTP POST to configured URL on task upsert |
| Tests | Store round-trip; e2e subscribe + persisted `GetTask` |

## Exit criteria

- Push config survives process restart (store-backed).
- `SubscribeToTask` streams current task state for `TASK_STATE_WORKING` tasks.
- Terminal tasks return `-32005` on subscribe.

**Target tag:** `v72.0.0`.
