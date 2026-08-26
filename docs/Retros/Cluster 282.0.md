# Cluster 282.0 retro — A2A JSON-RPC method names to spec

> Tag **`v282.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 1 of ~7 (full multi-transport + TCK).** No new gate tag.

## What shipped

The first, foundational step of the A2A v1.0 conformance arc: the JSON-RPC `method`
strings on `POST /a2a/v1/rpc` are now the **canonical A2A operation names** from the
spec's §5.3 Method Mapping Reference.

- `tasks/cancel` → **`CancelTask`**
- `tasks/pushNotificationConfig/set` → **`CreateTaskPushNotificationConfig`**
- `tasks/pushNotificationConfig/get` → **`GetTaskPushNotificationConfig`**
- Dropped the non-spec **`tasks/resubscribe`** alias (there is no resubscribe operation
  in A2A v1.0; `SubscribeToTask` is the only subscribe op).
- `SendMessage`, `SendStreamingMessage`, `GetTask`, `SubscribeToTask` were **already
  correct** and unchanged.

Constants renamed for accuracy (`METHOD_CANCEL_TASK`,
`METHOD_CREATE_PUSH_NOTIFICATION_CONFIG`); the Agent Card's advertised method list and
the integrator docs updated to match.

## Surprises / decisions

- **The backlog's premise was wrong, and grounding in the spec caught it before any
  code changed.** Open Work said "rename `SendMessage`→`message/send`". The *latest* A2A
  spec (verified against `a2aproject/A2A` `specification/a2a.proto` + the §5.3 mapping
  table in `docs/specification.md`) uses the proto-style operation names as the JSON-RPC
  method strings — so `SendMessage` was already right, and renaming it to `message/send`
  would have been a regression. The real gaps were three old-style names and a phantom
  `resubscribe`. This is exactly the rework the up-front spec fetch avoided.
- **Task-state enum already conforms.** Maidan's `TASK_STATE_WORKING`/`_COMPLETED`/… match
  the proto `TaskState` verbatim; no change needed.
- **Kept the cluster small.** Full A2A v1.0 conformance is a multi-cluster arc (the user
  chose the full multi-transport + TCK scope). This cluster is only the JSON-RPC
  method-name canonicalization — a clean, fully-testable unit. The missing operations,
  the Agent Card schema, and the REST/gRPC bindings each get their own cluster.
- **Pre-1.0 wire break, intended.** Renaming the method strings breaks any client using
  the old names; that is the point (aligning to the spec) and is allowed pre-1.0.

## Capability table extension

| Change | Where |
|--------|-------|
| A2A JSON-RPC method strings = canonical spec operation names (§5.3) | `crates/maidan-a2a/src/protocol.rs`, `crates/maidan-server/src/a2a_agent.rs` |
| Dropped non-spec `tasks/resubscribe`; renamed cancel + push-config-create methods | same |

## Risks identified + still open

- **Agent Card is still non-conformant** — it advertises a flat `capabilities: [method]`
  list, not the spec's `AgentCard` object (protocolVersion, capabilities object, skills,
  preferredTransport, additionalInterfaces). Its own cluster in the arc.
- **Missing operations** — `ListTasks`, `ListTaskPushNotificationConfigs`,
  `DeleteTaskPushNotificationConfig`, `GetExtendedAgentCard`. Next cluster. The current
  push-config store model is one-per-workspace; the spec is per-task with a configId, so
  List/Delete need a store change.

## Forward look

The A2A v1.0 arc (full multi-transport + TCK): **283** missing JSON-RPC operations +
per-task push-config model → **284** Agent Card §4.4.1 schema conformance → **285**
HTTP+JSON/REST binding (§11) → **286** gRPC binding (§10) → **287** transport negotiation
(§5.2) → **288** official A2A SDK/TCK interop CI. Each binding is a thin adapter over one
set of transport-agnostic operation handlers.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 281.0]]. Grounded in the authoritative A2A spec
(`a2aproject/A2A`: `a2a.proto` + §5.3 mapping) rather than the backlog's assumption.
