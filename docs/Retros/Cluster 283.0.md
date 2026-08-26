# Cluster 283.0 retro — A2A `ListTasks` + `GetExtendedAgentCard`

> Tag **`v283.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 2.** No new gate tag.

## What shipped

Two additive A2A JSON-RPC operations that the endpoint was missing, both wired into
the shared dispatch + advertised in the Agent Card:

- **`ListTasks`** — lists the authenticated workspace's A2A tasks, most-recently-updated
  first. New `Store::list_a2a_tasks(workspace_id, limit)` (both backends). Optional
  `contextId` filter and `pageSize` (default 50, clamped 1..=200). **Per-channel RBAC
  filter**: a task whose context thread the caller cannot read is dropped
  (`can_access_thread`), the same filtering discipline as the Cluster-162 aggregate
  reads. Single-page for now (`nextPageToken` always empty); the `status` filter and
  opaque page tokens are logged follow-ups.
- **`GetExtendedAgentCard`** — returns the Agent Card to an authenticated client
  (`MESSAGE_POST`). The card builder was refactored into a shared `agent_card_payload()`
  used by both the public `/.well-known/agent-card.json` route and this op.

## Surprises / decisions

- **`ListTasks` keys on `auth.workspace_id`.** Real A2A clients hold a workspace-scoped
  token, so that is the correct scope. A consequence: the bypass auth used by the
  existing `a2a_protocol_e2e` has no single workspace, so the new op's e2e is
  **auth-enabled** (a minted token in a real workspace) — added to `channel_access_e2e`
  where the auth-enabled A2A harness already lives, which also lets it assert the RBAC
  filter directly (a non-member of a private channel sees only the public task).
- **The task→thread link makes RBAC natural.** Each stored task carries a `contextId`
  (its thread); filtering listed tasks by `can_access_thread` on that thread reuses the
  existing per-channel model with no new concepts.
- **Kept the push-config ops out of this cluster.** `ListTaskPushNotificationConfigs`
  and `DeleteTaskPushNotificationConfig` need the per-task/`configId` push-config model
  (today it is one-config-per-workspace), which is a store schema change — its own
  cluster (284).

## Capability table extension

| Change | Where |
|--------|-------|
| A2A `ListTasks` op (+ `Store::list_a2a_tasks`, both backends; RBAC-filtered, `contextId`/`pageSize`) | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-store/src/{postgres,sqlite}/a2a.rs`, `crates/maidan-server/src/a2a_agent.rs` |
| A2A `GetExtendedAgentCard` op (shared `agent_card_payload()`) | `crates/maidan-server/src/a2a_agent.rs` |

## Risks identified + still open

- **Push-config model is still one-per-workspace** — `Create`/`Get` work; `List`/`Delete`
  + `configId` semantics need the per-task model (cluster 284).
- **Agent Card is still the flat non-spec shape** — §4.4.1 schema conformance (cluster
  285).
- **`ListTasks` is single-page** — `pageSize` limits results but `status` filter and
  `nextPageToken` pagination are not implemented; logged.

## Forward look

Arc continues: **284** per-task push-config model + `List`/`Delete` ops → **285** Agent
Card §4.4.1 schema → **286** HTTP+JSON/REST binding → **287** gRPC binding → **288**
transport negotiation → **289** official A2A SDK/TCK interop CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 282.0]]. Shapes grounded in the A2A `a2a.proto` (`ListTasksRequest`/
`Response`, `GetExtendedAgentCardRequest`).
