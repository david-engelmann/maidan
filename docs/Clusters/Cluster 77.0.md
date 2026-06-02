# Cluster 77.0 — HTTP capability map complete

**Theme:** Every documented HTTP operation has a declared required capability; CI fails on drift between `openapi.json`, the contract file, and handler checks.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XIV · tag **`v77.0.0`**.

**Predecessor:** Cluster **69** shipped MCP capability map + **sample** HTTP routes in `contracts/http-capability-routes.json` (six cases) and `http_capability_map_contract` (known capability strings only).

---

## Problem

Integrators and operators cannot assume “if it is in OpenAPI, the capability story is tested.” Today:

- `GET /openapi.json` (utoipa `ApiDoc`) documents ~70 operations but **omits** several shipped routes (e.g. workspace context, automation DLQ/replay, agent apps).
- `contracts/http-capability-routes.json` covers **six** table-driven denial samples for `capability_matrix_e2e`.
- Handlers call `cap(auth, …)` ad hoc; there is **no** single registry that CI can diff against OpenAPI.

Cluster **69** closed the MCP side; **77** closes the HTTP side to the same bar.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Audit** | Script or test that lists `(method, path)` from `ApiDoc::openapi()` and from the axum router manifest; report gaps |
| **OpenAPI** | Add missing path stubs for shipped protected routes (automation, apps, workspace context, token quotas if exposed) so public doc matches reality |
| **Contract** | Expand `contracts/http-capability-routes.json` (or add `http-capability-map.json` keyed by `operationId`) with **every** protected HTTP operation + required capability |
| **CI** | `http_openapi_capability_contract` (name TBD): OpenAPI operations ⊆ contract; contract capabilities ⊆ known set; optional: contract paths resolve in router |
| **E2E** | Extend `capability_matrix_e2e` to iterate contract rows (deny without cap, allow with cap) like `mcp_capability_matrix_e2e` |
| **Docs** | [[Capability Map]] + [[Agent Integration]] — HTTP section lists generation/CI; note exclusions (bootstrap, health, metrics, OIDC browser, unauthenticated well-known) |

### Capability assignment rules (normative for this cluster)

Reuse strings from `maidan_auth::capability` (same as MCP map):

| Capability | HTTP (default rule) |
|------------|---------------------|
| `workspace:read` | GET/HEAD on workspace-scoped resources not covered below |
| `workspace:write` | POST/PUT/PATCH/DELETE on workspace resources; purge; automation replay; hook/webhook CRUD |
| `message:post` | POST messages; PATCH own message |
| `thread:transition` | POST `/threads/:id` FSM transition |
| `artifact:upload` | POST/PUT artifact upload paths |
| `search:query` | GET `.../search` |
| `event:subscribe` | WS `/ws/subscribe` (document in contract appendix, not OpenAPI path) |
| `token:admin` | Mint/revoke API tokens |
| `federation:ingest` / `federation:admin` | A2A ingest + peer CRUD (peer bearer) |

Document explicit exceptions (e.g. message edit by non-author requires `workspace:write`) in the contract row `notes` field.

---

## Non-goals

- MCP tool map changes (frozen at **69** unless a path rename forces a doc touch).
- MCP streamable / A2A JSON-RPC capability tables (**78**, **79**).
- Changing capability strings or adding new capabilities (defer to auth cluster if needed).
- Proving handler-level auth for **UI session cookie** routes beyond documenting which use session vs bearer (session may reuse member caps via `mint_auth_session_token`).

---

## Out of OpenAPI but in contract appendix

These must appear in the machine-readable map with a `surface` field, but are not required to be utoipa paths in **77** unless cheap:

| Surface | Examples |
|---------|----------|
| `websocket` | `GET /ws/subscribe` |
| `mcp` | `POST /mcp`, `GET /mcp/stream`, streamable session |
| `a2a` | `POST /a2a/v1/rpc` methods |
| `public` | `/health/*`, `/.well-known/*`, `/metrics`, bootstrap (document as `capability: none` + env gate) |

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 77.0.1 | `test(server): openapi vs router route audit` |
| 77.0.2 | `feat(openapi): document missing HTTP operations` |
| 77.0.3 | `feat(contracts): full http capability map + openapi parity test` |
| 77.0.4 | `test(server): table-driven http capability_matrix from contract` |
| 77.0.5 | `docs: Capability Map + Agent Integration HTTP CI` |
| 77.0.retro | `docs(retro): Cluster 77.0 + v77.0.0 tag prep` |

**Ordering:** 77.0.1 before 77.0.3; 77.0.2 can parallel 77.0.1 once gaps are listed; 77.0.4 after 77.0.3.

---

## Exit criteria

- `contracts/http-capability-routes.json` (or successor file) lists **every** protected OpenAPI operation with a required capability.
- CI fails if `openapi.json` gains/loses an operation without updating the contract.
- `capability_matrix_e2e` (or sibling) exercises **all** contract HTTP rows for deny/allow.
- [[Capability Map]] states HTTP parity is complete (MCP was already complete at **69**).
- **`v77.0.0`** tagged after retro.

---

## Risks

| Risk | Mitigation |
|------|------------|
| utoipa path templates vs axum `{id}` mismatch | Normalize to OpenAPI `{param}` form in tests |
| Cookie/session routes hard to table-drive | Start with bearer-only rows; session follows same caps in docs |
| Large e2e runtime | One token pair per capability class; reuse workspace setup |

---

## References

- [[Clusters/Product Ladder 77+]], [[Retros/Cluster 69.0]]
- `contracts/http-capability-routes.json`, `crates/maidan-server/tests/http_capability_map_contract.rs`
- `crates/maidan-server/tests/capability_matrix_e2e.rs`, `scripts/check-agent-contract.sh`
- [[Capability Map]], [[Agent Integration]]
