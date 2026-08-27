# Cluster 286.0 retro — A2A HTTP+JSON/REST binding

> Tag **`v286.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1: A2A
> v1.0 compliance, arc part 5.** No new gate tag.

## What shipped

The A2A HTTP+JSON/REST binding (§11): the same operations as the JSON-RPC endpoint,
now reachable as REST routes under `/a2a/v1`, implemented as **thin adapters** over the
existing `dispatch_*` handlers (no business-logic duplication).

- **Routes** (9 request/response ops): `POST /a2a/v1/message:send`, `GET /a2a/v1/tasks`,
  `GET /a2a/v1/tasks/{id}`, `POST /a2a/v1/tasks/{id}:cancel`,
  `POST|GET /a2a/v1/tasks/{id}/pushNotificationConfigs`,
  `GET|DELETE /a2a/v1/tasks/{id}/pushNotificationConfigs/{configId}`,
  `GET /a2a/v1/extendedAgentCard`.
- **`rest_response`** converts the op's `JsonRpcResponse` to HTTP: `result` → `200` +
  the result JSON; error → an HTTP status for the JSON-RPC code (§5.4-style). Each
  handler builds the operation's params from the path/query/body and calls the shared
  dispatch fn with a dummy id.
- The Agent Card now advertises the REST interface as a second `supportedInterfaces`
  entry (`protocolBinding: "HTTP+JSON"`, url `/a2a/v1`).

## Surprises / decisions

- **The `:action` custom-method paths route fine on axum 0.7.** axum 0.7 uses `:param`
  as the capture sigil, which made `/message:send` and `/tasks/{id}:cancel` look risky. A
  standalone matchit-0.7.3 probe proved it: a literal `:` mid-segment is fine
  (`/message:send` matches literally), and `/tasks/:id` captures `"uuid:cancel"` whole —
  so the cancel route captures the segment and splits on `':'` (task UUIDs have no colon).
  `GET` and `POST` on `/tasks/:id` share the `:id` param name (matchit requires it).
- **Error mapping is Maidan's codes, not the full spec taxonomy.** Maidan overloads
  `-32001` for auth/capability failures, so it maps to `403` (not the spec's `404`
  TaskNotFound); `-32602` (params/not-found) → `400`; `-32603` → `500`. Aligning to the
  full A2A error set is a logged follow-up.
- **Streaming REST endpoints deferred.** `message:stream` and `tasks/{id}:subscribe`
  return SSE; the dispatch fns already return a `Response`, so they're an easy follow-up,
  kept out of this cluster to bound scope.
- **No matrix body-clause preflight needed.** A2A routes carry `surface: "a2a"` in the
  capability-map, which the `http_capability_matrix_e2e` skip-list excludes (auth is
  enforced inside the dispatch handlers, like the JSON-RPC endpoint). Added the 9 map
  entries for completeness.

## Capability table extension

| Change | Where |
|--------|-------|
| A2A HTTP+JSON/REST binding (9 routes, thin adapters + `rest_response` converter) | `crates/maidan-server/src/a2a_agent.rs`, `crates/maidan-server/src/app.rs` |
| Agent Card advertises the REST interface (2nd supportedInterfaces entry) | `crates/maidan-server/src/a2a_agent.rs` |
| Capability-map entries for the REST routes (surface "a2a") | `contracts/http-capability-map.json` |

## Risks identified + still open

- **Streaming REST endpoints deferred** (`message:stream`, `tasks:subscribe`).
- **Error taxonomy** is Maidan's JSON-RPC codes, not the full A2A error set.
- **REST routes not in OpenAPI** (like the JSON-RPC endpoint + `/ui/api`); the generated
  OpenAPI doesn't document the A2A surface.

## Forward look

Arc continues: **287** gRPC binding (§10) — tonic server compiling `a2a.proto` on a
second port (new deps → cargo-deny review, compose/Helm wiring) → **288** transport
negotiation + configurable public origin (§5.2) → **289** official A2A SDK/TCK interop CI.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 285.0]]. REST path mapping from the A2A `a2a.proto` `google.api.http`
annotations + the §5.3 mapping table.
