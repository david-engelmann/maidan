# Cluster 284.0 retro — A2A per-task push notification configs

> Tag **`v284.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 3.** No new gate tag.

## What shipped

The A2A push-notification model moved from Maidan's one-config-per-workspace shortcut
to the spec's **per-task, multi-config with a stable `configId`** model, completing the
four push-config operations:

- New table `maidan_a2a_task_push_configs (task_id, config_id, push_url)` (pg 0049 /
  sqlite 0048) + four store methods on both backends: `create`/`get`/`list`/`delete`
  (upsert by `(task_id, config_id)`; delete reports whether a row was removed).
- The JSON-RPC ops now operate per-task: **`CreateTaskPushNotificationConfig`**
  (server generates the `configId` when the client omits it, returns the stored
  config), **`GetTaskPushNotificationConfig`** (`taskId` + `id`),
  **`ListTaskPushNotificationConfigs`** (`taskId`), **`DeleteTaskPushNotificationConfig`**
  (`taskId` + `id`, errors if none). Each loads the task and runs
  `ensure_task_workspace_access` for RBAC.
- **Push delivery rewired**: when a task is persisted, notifications now fan out to
  **all of that task's** push configs (`list_a2a_task_push_configs(task.id)`), not a
  single workspace-level URL.
- Agent Card advertises `List`/`Delete` push-config ops.

## Surprises / decisions

- **Kept the old workspace-level table as harmless dead code.** The previous
  `maidan_a2a_push_configs (workspace_id, push_url)` table and its `upsert`/`get` store
  methods are no longer used by any op or the delivery path, but removing them would
  churn an existing store test for no functional gain (migrations are append-only, so
  the table can't be dropped without another migration). They stay as pub trait methods
  (no dead-code warning) with cleanup logged.
- **`configId` is server-generated.** A client may supply an `id`; if absent (or blank),
  the server assigns a UUID and returns it — matching the spec's "unique identifier for
  this configuration".
- **RBAC via the task, not the workspace.** Each push-config op loads the task and calls
  `ensure_task_workspace_access` (workspace + the task's context-thread channel access),
  so a caller can only manage push configs for tasks it can reach.
- **Captured `url` only.** The proto `TaskPushNotificationConfig` also has `token` and
  `authentication`; this cluster stores the `url` (the delivery target). Token/auth are
  a logged follow-up.

## Capability table extension

| Change | Where |
|--------|-------|
| Per-task push-config table + store CRUD (both backends) | `migrations/{postgres/0049,sqlite/0048}_a2a_task_push_configs.sql`, `crates/maidan-store/src/{store.rs,postgres/a2a.rs,sqlite/a2a.rs,postgres/mod.rs,sqlite/mod.rs}` |
| `Create`/`Get`/`List`/`Delete` TaskPushNotificationConfig JSON-RPC ops (per-task, RBAC-checked) + delivery fan-out | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-server/src/a2a_agent.rs` |

## Risks identified + still open

- **Old workspace-level push table/methods are dead code** — cleanup deferred (needs a
  drop migration + touching the old store test).
- **`token`/`authentication` fields not stored** — url-only delivery; logged.
- **No pagination** — `ListTaskPushNotificationConfigs` returns all configs
  (`nextPageToken` empty).

## Forward look

Arc continues: **285** Agent Card §4.4.1 schema conformance → **286** HTTP+JSON/REST
binding → **287** gRPC binding → **288** transport negotiation → **289** official A2A
SDK/TCK interop CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 283.0]]. Shapes grounded in the A2A `a2a.proto`
(`TaskPushNotificationConfig` + the §5.3 mapping).
