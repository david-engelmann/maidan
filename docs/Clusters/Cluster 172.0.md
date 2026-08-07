# Cluster 172.0 — agentic: MCP structured backpressure

**Theme:** Arc 3 (agentic features), part 2 — a typed, machine-readable
rate-limit signal for MCP JSON-RPC clients.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v172.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `McpError::RateLimited { retry_after_ms }` → JSON-RPC error (code `-32029`, `data.retry_after_ms`) | `maidan-mcp/src/error.rs` |
| Rate-limit rejections on `/mcp` + `/mcp/streamable` return a JSON-RPC error envelope (429 + `Retry-After`) | `maidan-server/src/rate_limit/mod.rs`, `quota.rs` |

## Why

The rate limiter is HTTP middleware that returns a `problem+json` **429**
*before* the request reaches the MCP dispatcher. An agentic MCP client speaks
JSON-RPC — it gets an opaque transport 429 with no in-band, typed way to learn
*how long* to back off. Now a throttled `POST /mcp` (and `/mcp/streamable`)
returns a JSON-RPC error envelope:

```json
{"jsonrpc":"2.0","id":null,
 "error":{"code":-32029,"message":"rate limited; retry after 60000ms",
          "data":{"retry_after_ms":60000}}}
```

still under HTTP 429 + a `Retry-After` header, so HTTP infra sees it too. A
client that recognizes `-32029` backs off for `retry_after_ms` and retries.

## Key decisions

- **429 status + JSON-RPC body.** Keep the 429 (HTTP infra / proxies see
  backpressure) but shape the body as JSON-RPC so the agent's RPC layer gets a
  typed signal. `id` is `null` — the middleware rejects before parsing the
  request id, which JSON-RPC permits for undeliverable-id errors.
- **`-32029`** in the server-defined range (`-32000..=-32099`), distinct from the
  existing MCP error codes.
- **Covers the quota limiter too.** `quota.rs` (per-token-capability quotas)
  shares `too_many`, so a quota rejection on an MCP path is also JSON-RPC-shaped.
- **`/mcp/stream` (GET SSE) is not covered** — a JSON-RPC body doesn't fit an SSE
  connection reject; only the JSON-RPC POST endpoints get the envelope.

## Non-goals

- Moving rate limiting into the MCP dispatcher (the middleware stays the single
  source of truth; only the response *shape* is path-aware).
- Adaptive/priority backpressure — out of scope.

## Exit criteria

- A throttled MCP POST returns `-32029` + `retry_after_ms`; non-MCP routes keep
  the existing `problem+json`; suites green — **met**.
- `v172.0.0` tagged.

## Verification & limits

- `mcp_rate_limit_returns_jsonrpc_backpressure_envelope` (exceed
  `MAIDAN_RATE_LIMIT_MAX` on `POST /mcp` → assert 429 + `Retry-After` +
  `error.code == -32029` + `error.data.retry_after_ms`). `error.rs` unit tests
  cover the variant → code/data mapping. The existing non-MCP 429 test
  (`burst_over_limit_returns_429_problem_json`) still asserts the `problem+json`
  shape, proving the branch is path-scoped.
- Limit: `retry_after_ms` is the full window (fixed-window limiter), not the
  precise time to the next token — a safe upper bound.

## References

- [[Retros/Cluster 172.0]]; `maidan-mcp/src/error.rs`,
  `maidan-server/src/rate_limit/mod.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.
