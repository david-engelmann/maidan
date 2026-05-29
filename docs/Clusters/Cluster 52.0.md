# Cluster 52.0 — FSM automation hooks

**Theme:** Invoke registered handlers when a thread transitions between FSM states.

## Problem

Agents and operators need automation when threads move through the lifecycle
(`open` → `in_review` → `closed`, etc.) without polling the event log.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Store | `maidan_fsm_hooks` with optional `from_state` / `to_state` filters (NULL = wildcard) |
| HTTP | `POST/GET/DELETE /workspaces/:wid/fsm-hooks` |
| Dispatch | `FsmHookWorker` on `EventKind::ThreadStateChanged` — signed HTTP or MCP tool |
| MCP | `register_fsm_hook`, `list_fsm_hooks` tools |

## Tag

`v52.0.0`

See [[Clusters/Product Ladder 35+]] Phase V.
