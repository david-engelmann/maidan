# Cluster 52.0 retro — FSM automation hooks

> Tag **`v52.0.0`**.

## What shipped

- `maidan_fsm_hooks` table + CRUD on `/workspaces/:wid/fsm-hooks`.
- `FsmHookWorker` subscribes to `ThreadStateChanged` on the event bus and dispatches matching hooks.
- HTTP handlers receive `thread_state_changed` JSON with `X-Maidan-Signature` / `X-Maidan-Event` (same signing as webhooks/slash).
- MCP handlers invoked with `AuthContext::bypass()` for the configured tool name.
- MCP tools `register_fsm_hook` and `list_fsm_hooks`.

## What was deferred

- Delivery queue / retries for failed HTTP hook invocations (webhooks-style).
- Hook invocation audit table and operator replay UI.
- Inline dispatch on `transition_thread` (bus-only keeps federation-ingested transitions covered).

## Forward look

Cluster **53**: Workspace full erasure (Phase VI).
