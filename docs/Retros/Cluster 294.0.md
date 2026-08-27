# Cluster 294.0 retro — TypeScript SDK (0.1.0)

> Tag **`v294.0.0`**. Phase XXIV (post-gate hardening). SDK arc, part 1. No new gate tag.

## What shipped

The first usable language client, built to the frozen v1 contract
([[Client Contract]]) and verified black-box against a running server:

- **`sdk/typescript/index.js`** — a dependency-free `Client` (REST + WebSocket; uses the
  global `fetch` on Node 18+ and a pluggable WebSocket, global on Node 22+ / browser or
  injected via `options.WebSocket`). Namespaced surface (`workspaces`, `channels`,
  `threads`, `messages`, `artifacts`), the hero `claimNextThread` / `renewClaim`,
  `subscribe(filter, onEvent)`, and the `waitFor{Result,Mention,Ready}` helpers that wrap
  it. Constructor `(baseUrl?, token?, options?)` defaults from `MAIDAN_URL` / `MAIDAN_TOKEN`;
  `client.mcpUrl` is `{base}/mcp/streamable` (a string — no MCP dependency).
- **`sdk/typescript/index.d.ts`** — full type declarations: branded ID types
  (`WorkspaceId`/`ChannelId`/… with an *optional* brand so plain strings stay assignable),
  `ClientOptions`, `MaidanError`, `Subscription`, `EventFrame`, and the `Client` class.
- **`MaidanError`** — one error type carrying `.status` + the parsed `.body`, with
  `.isConflict` (409) / `.isForbidden` (403) / `.isRateLimited` (429) and `.retryAfter`
  (seconds, from `Retry-After` on a 429).
- **`sdk/typescript/test.mjs`** — a `node --test` black-box suite (hero loop; `getResult`
  404 on an unset thread; claim-next; error surfacing; WS subscribe delivers a posted
  message).
- **`scripts/sdk-test.sh`** — a language-agnostic harness (`typescript`/`python`/`go`/`rust`)
  that builds + boots a source Maidan on SQLite (auth disabled), waits for health, runs the
  chosen suite against it, and tears down. The Python/Go/Rust arms are stubs for 295–297.
- **`package.json`** bumped 0.0.1 → **0.1.0** (homepage → the docs site, `engines.node >= 18`,
  `test` = `node --test`); **`README.md`** rewritten with real usage.

Verified locally: **5/5 tests pass** against a source-built server.

## Surprises / decisions

- **`setResult` write can't be tested under the auth-disabled harness.** `produced_by` is a
  NOT-NULL FK to a member, and `AUTH_DISABLED` is *unconditional* bypass (nil member, ignores
  any presented bearer — `auth.rs:32`), so a write 500s on the FK. Minting a real token
  doesn't help (bypass ignores it). The server's own auth-enabled `thread_result_e2e` proves
  the write; the SDK test exercises the result *route* + client error path via `getResult` →
  404 on an unset thread. The SDK test's job is to prove the *client* builds correct requests,
  not to re-prove server routes.
- **Build-then-run in the harness**, not `cargo run` — the Cluster-290 lesson (a cold compile
  outlasts the health-wait). `cargo build` blocks, then the binary boots fast.
- **`MAIDAN_BIND`, not `MAIDAN_PORT`** for the port (the recurring env-name gotcha).
- **Branded IDs with an optional brand.** `type WorkspaceId = string & { readonly __maidan?: "workspace" }`
  conveys intent in editors while keeping plain strings assignable — usable now; stricter
  enforcement is a future refinement, not a 0.1.0 blocker.
- **Member creation is not in the SDK surface** (seeded via bootstrap/CLI) — the black-box
  test seeds one over the raw bootstrap route, matching `examples/a2a_interop.py`.

## Capability table extension

New TypeScript client (`sdk/typescript`, 0.1.0) + a black-box SDK test harness
(`scripts/sdk-test.sh`). No server capability change.

## Risks identified + still open

- **Not published to npm.** Publishing needs an `NPM_TOKEN` repo secret + a release trigger
  (an `sdk-*` tag); logged in [[Open Work]] under the standing publish authorization.
- **No SDK interop CI yet** — the harness is local-only until the four SDKs exist; a CI job is
  a follow-up once 295–297 land.
- The frozen v1 contract omits streaming REST + the full A2A/gRPC surface (deferred by design).

## Forward look

The SDK arc continues: **295 Python → 296 Go → 297 Rust**, each to a usable 0.1.0 against
the same frozen contract + a black-box suite run through `scripts/sdk-test.sh`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 293.0]].
Built under the standing SDK-arc authorization (usable 0.1.0, all four languages).
