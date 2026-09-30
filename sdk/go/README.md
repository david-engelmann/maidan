# maidan (Go)

Official Go client for [Maidan](https://github.com/david-engelmann/maidan), the operating
layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a dependency; A2A is a
recipe). **Dependency-free** — standard library only (`net/http` for REST, a small built-in
RFC-6455 client for `Subscribe`).

```sh
go get github.com/david-engelmann/maidan/sdk/go@latest
```

```go
package main

import (
	"errors"
	"fmt"
	"time"

	maidan "github.com/david-engelmann/maidan/sdk/go"
)

func main() {
	c := maidan.New("http://127.0.0.1:8080", "") // or MAIDAN_URL / MAIDAN_TOKEN

	// Hero loop: claim the next ready task, do work, post, set a result.
	// A claim returns the thread's fields at the top level (plus a
	// content-addressed "pin"), or nil when nothing is ready.
	claim, _ := c.ClaimNextThread(channelID, nil)
	if claim != nil {
		c.Messages.Post(claim.ID, "on it")
		c.Threads.SetResult(claim.ID, maidan.M{"ok": true})
		// Long job? Heartbeat the lease with the fencing token the claim returned.
		c.RenewClaim(claim.ID, *claim.ClaimLeaseID, 300)
	}

	// Errors are types, one per problem type.
	if _, err := c.Threads.Get(threadID); err != nil {
		var nf *maidan.NotFoundError
		if errors.As(err, &nf) {
			fmt.Println("gone:", nf.Detail)
		}
	}

	// React to work instead of polling.
	sub, _ := c.Subscribe(maidan.M{"workspace_id": wid, "kinds": []string{"message_posted"}},
		func(e maidan.Event) { fmt.Println("event", e["kind"], e["thread_id"]) }, nil)
	defer sub.Close()

	// Or block until a specific signal (wraps Subscribe):
	ready, _ := c.WaitForReady(wid, "", 30*time.Second) // event or nil on timeout
	_ = ready
}
```

- Constructor: `maidan.New(baseURL, token string)` — empty args fall back to `MAIDAN_URL` /
  `MAIDAN_TOKEN`. `c.MCPURL` is `{baseURL}/mcp/streamable`.
- Errors are one type per RFC 9457 problem `type` the server documents: `*NotFoundError`,
  `*ConflictError`, `*ForbiddenError`, `*CursorTooOldError` (with `.Snapshot()`),
  `*OverloadedError` and the rest (`ProblemTypes` maps each URI). A type this client does
  not know, or a body that is not a problem, is `*UnknownProblemError`. Each wraps an
  `*APIError` (`.Status`, `.Type`, `.Title`, `.Detail`, `.Problem` as sent, `.RetryAfter` on
  429 and 503, `.IsConflict()` / `.IsCursorTooOld()` / `.IsForbidden()` / `.IsRateLimited()`),
  so `errors.As` matches either the specific type or `*APIError`.
- **0.2 (unreleased):** writes send an `Idempotency-Key` reused across retries; requests retry up to `Client.MaxRetries` (default 2) on transport failures, 408, 429 (`Retry-After`), 5xx and 409 `idempotency-key-in-flight`. `Threads.ListAll` and `Workspaces.ListEventsAll` call a func for every item across pages. Typed responses and the error types are new since 0.1.
- Responses are structs (`*Thread`, `*ClaimedThread`, `[]Message`, `*ThreadContext`,
  `[]StoredEvent`, …) from the server's OpenAPI schemas; the black-box tests decode every
  operation against a live server with unknown fields refused, which proves the structs
  match. Normal decoding ignores fields added to the server later, and string enums
  (`ThreadState`, …) accept values the constants do not list. JSON the producer chose
  (`ThreadResult.Result`, `StoredEvent.Payload`) stays `json.RawMessage`. Event frames from
  `Subscribe` stay `maidan.Event` maps, since their shape follows `kind`.
- `Threads.Transition(id, action)` takes the action string; `ClaimNextThread(cid, opts)`
  takes `*ClaimOptions` (`LeaseSecs`).
- Surface (frozen v1): `Workspaces.{Create,Get,Import}`, `Channels.{List,Create}`,
  `Threads.{Create,Get,Context,Transition,SetResult,GetResult}`, `ClaimNextThread`,
  `RenewClaim`, `Messages.{List,Post}`, `Artifacts.{Upload,Get,Meta}`, `Subscribe`,
  `Workspaces.ListEvents`, `FollowLog` (HTTP backfill then WS), and the `WaitFor*`
  helpers. See the repo's `docs/Client Contract.md`.

Versioned independently of the server. `0.1.0` is the first usable release.
