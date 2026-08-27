# Cluster 296.0 retro — Go SDK (0.1.0)

> Tag **`v296.0.0`**. Phase XXIV (post-gate hardening). SDK arc, part 3. No new gate tag.

## What shipped

The third usable language client, to the frozen v1 contract ([[Client Contract]]),
verified black-box through the Cluster-294 harness:

- **`sdk/go/client.go`** — a `Client` (REST + WebSocket), **dependency-free (stdlib only)**:
  REST rides `net/http`. Service structs mirror the surface (`Workspaces`/`Channels`/
  `Threads`/`Messages`/`Artifacts`, PascalCase-exported per Go idiom), plus the hero
  `ClaimNextThread` / `RenewClaim`. Object responses come back as `maidan.M` (`map[string]any`)
  and lists as `[]maidan.M`, so unknown fields are preserved and ignored (forward-compat).
  `c.MCPURL` is `{base}/mcp/streamable`.
- **`sdk/go/ws.go`** — a small hand-rolled RFC-6455 WebSocket client (dial + handshake over
  `net`/`crypto/tls`, one masked send, a receive loop with auto-pong/close/fragmentation),
  `Subscribe(filter, onEvent, onError)` returning a `*Subscription` with `Close()`, and the
  `WaitFor{Result,Mention,Ready}` helpers (wrap `Subscribe` via a channel + `time.After`).
- **`APIError`** — `.Status`, `.Body`, `.RetryAfter` (429), `.IsConflict()` / `.IsForbidden()` /
  `.IsRateLimited()`; matched with `errors.As`.
- **`sdk/go/client_test.go`** — a `go test` black-box suite (hero loop; `GetResult` 404 on an
  unset thread; claim-next; error surfacing; WS subscribe), gated on `MAIDAN_URL` (skips when
  unset). **All pass** locally; `go vet` + `gofmt` clean.
- **`README.md`** rewritten with real usage; `go.mod` stays at module
  `github.com/david-engelmann/maidan/sdk/go` (0.1.0 — Go modules version by tag).

Verified locally: **all tests pass** against a source-built server; `go vet` + `gofmt -l` clean.

## Surprises / decisions

- **Hand-rolled WebSocket again.** Go's stdlib has no WebSocket client (the common ones —
  `gorilla/websocket`, `nhooyr.io/websocket` — are third-party). Consistent with the TS/Python
  clients' zero-dependency stance, `Subscribe` ships a small stdlib RFC-6455 client over a
  `bufio.Reader` (which cleanly holds any handshake over-read as the start of the frame stream).
  The black-box test drives the real handshake + a real `message_posted` frame, so a framing
  bug fails the test.
- **`maidan.M` for responses**, not generated typed structs. For 0.1.0, returning
  `map[string]any` keeps the client lean, honors "ignore unknown fields", and avoids typed-model
  drift against the server; typed models are a logged future refinement (the TS `.d.ts` made the
  same `any`-return call).
- **Separate Go module** (`sdk/go/go.mod`) with **no dependencies** → no `go.sum`. It sits
  outside the Rust workspace and outside any CI job today (SDK interop CI is a follow-up once
  all four exist).
- Same **result-write constraint** as TS/Python: under the auth-disabled harness the acting
  member is nil (`produced_by` FK), so the test exercises the result route via `GetResult` → 404.

## Capability table extension

New Go client (`sdk/go`, 0.1.0), dependency-free (stdlib REST + a hand-rolled WS). No server
capability change.

## Risks identified + still open

- **Not published / tagged for `go get` by version** — Go modules are consumed by tag; a
  published, importable `sdk/go` version needs an `sdk-*` (or module-path) tag; logged in
  [[Open Work]].
- Responses are untyped (`map[string]any`) — typed models are a future refinement.

## Forward look

The SDK arc concludes with **297 Rust** (usable 0.1.0 against the same contract + a black-box
suite; the Rust client must not depend on `maidan-server`).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 295.0]].
Built under the standing SDK-arc authorization (usable 0.1.0, all four languages).
