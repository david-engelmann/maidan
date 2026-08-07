# Cluster 172.0 retro — MCP structured backpressure

> Tag **`v172.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 3 (agentic features), part 2.

## What shipped

- `McpError::RateLimited { retry_after_ms }` → a JSON-RPC error with code
  `-32029` and `data.retry_after_ms`.
- The rate-limit middleware (and the token-quota limiter, which shares
  `too_many`) now returns that JSON-RPC error envelope for `POST /mcp` and
  `POST /mcp/streamable` — still HTTP 429 + `Retry-After`, but a JSON-RPC body so
  an agent's RPC layer gets a typed, machine-readable backoff signal.

## What was deferred / not covered

| Item | Why |
|------|-----|
| `/mcp/stream` (GET SSE) | A JSON-RPC body doesn't fit an SSE connection reject. |
| Rate limiting inside the dispatcher | The middleware stays the single source of truth; only the response *shape* is path-aware. |
| Adaptive / priority backpressure | Out of scope. |

## Surprises

- **The signal existed, the shape didn't.** The 429 already carried a
  `Retry-After` header — the gap was purely that MCP clients speak JSON-RPC and
  never saw a typed error. So this was a small, surgical response-shaping change,
  not new plumbing. No new capability, route, contract, or catalog entry.
- **Two limiters, one exit.** Both the global/per-workspace middleware and the
  per-token-capability quota check funnel through `too_many`, so path-awareness
  had to live there (threaded a `is_mcp` bool) to cover both.

## Decisions

- **429 + JSON-RPC body, `id: null`.** Keep the HTTP status for infra; add the
  JSON-RPC body for the agent. The middleware rejects before parsing the request
  id, so the error uses `id: null` (permitted for undeliverable-id errors).
- **`-32029`** — server-defined range, distinct from the existing codes.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP rate-limit → JSON-RPC backpressure envelope (`-32029` + `retry_after_ms`) | `maidan-mcp/src/error.rs`, `maidan-server/src/rate_limit/mod.rs` |

## Risks identified + still open

- **Low.** Purely additive + path-scoped: non-MCP routes keep `problem+json`
  (guarded by the retained `burst_over_limit_returns_429_problem_json` test); the
  MCP branch is new. `retry_after_ms` is the whole fixed window (a safe upper
  bound), not the precise next-token time.

## Forward look

Arc 3 continues: **structured message content** (typed blocks over
`body`/`metadata` — the larger flagship, likely with a Plan-agent pre-map like
171) and **HITL approvals** over the elicitation transport (built in 145–148/154).
Then arc 4 (token round 3).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].
