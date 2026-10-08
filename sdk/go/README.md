# maidan (Go)

Official Go client for [Maidan](https://github.com/david-engelmann/maidan), the operating
layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a dependency; A2A is a
recipe). **Dependency-free** — standard library only (`net/http` for REST, a small built-in
RFC-6455 client for `Subscribe`).

The example below is 0.3.0, which is not tagged yet: `@latest` still resolves to 0.1.0,
which returns maps and has one error type. Until 0.3.0 is tagged, take `main`:

```sh
go get github.com/david-engelmann/maidan/sdk/go@main
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
- **0.3.0 (unreleased; 0.2.0 was never tagged):** writes send an `Idempotency-Key` reused across retries; requests retry up to `Client.MaxRetries` (default 2) on transport failures, 408, 429 (`Retry-After`), 500, 502, 503, 504 and 409 `idempotency-key-in-flight`. `Threads.ListAll` and `Workspaces.ListEventsAll` call a func for every item across pages, asking for at most `MaxPageSize` (500, the server's cap) per page. Typed responses and the error types are new since 0.1.
- Responses are structs (`*Thread`, `*ClaimedThread`, `[]Message`, `*ThreadContext`,
  `[]StoredEvent`, …) from the server's OpenAPI schemas; the black-box tests decode every
  operation against a live server with unknown fields refused, which proves the structs
  match. Normal decoding ignores fields added to the server later, and string enums
  (`ThreadState`, …) accept values the constants do not list. JSON the producer chose
  (`ThreadResult.Result`, `StoredEvent.Payload`) stays `json.RawMessage`. Event frames from
  `Subscribe` stay `maidan.Event` maps, since their shape follows `kind`.
- `Threads.Transition(id, action)` takes the action string; `ClaimNextThread(cid, opts)`
  takes `*ClaimOptions` (`LeaseSecs`).
- Usage (0.3.0): `NormalizeUsage(provider, body, UsageOptions{Model, Provider})` turns an Anthropic, Bedrock Converse, OpenAI Responses or Chat Completions, Gemini, DeepSeek, Mistral, xAI or vLLM response body into a `NormalizedUsage` (the `model`, `tokens` and `evidence` of a `report_usage` body), and `USDMicros(tokens, price)` is the charge the server checks. A body it cannot read returns a `*UsageError`. `input` comes out uncached and cache writes split into 5-minute and 1-hour tiers, as the ledger counts them. See "Normalizing provider usage" in the repo's `docs/Integration.md`.
- Surface (frozen v1): `Workspaces.{Create,Get,Import}`, `Channels.{List,Create}`,
  `Threads.{Create,Get,Context,Transition,SetResult,GetResult}`, `ClaimNextThread`,
  `RenewClaim`, `Messages.{List,Post}`, `Artifacts.{Upload,Get,Meta}`, `Subscribe`,
  `Workspaces.ListEvents`, `FollowLog` (HTTP backfill then WS), and the `WaitFor*`
  helpers. See the repo's `docs/Client Contract.md`.
- Caching (0.3.0): `c.Channels.Boot(cid)` returns the channel's boot prefix as served, with its SHA256 (for `evidence.pack_sha256`). `CachedPrefix(provider, text, ttl)` places it with a cache breakpoint, `CacheKey(workspaceID, group)` and `CacheKeyFields(provider, key)` give one cache key per shared-prefix group, never shared across workspaces, and `GatewaySession(gateway, threadID, path, name)` passes the thread id as an OpenRouter, Helicone, LiteLLM or TensorZero session id. See the repo's `docs/Harness Caching.md` for where each harness puts Maidan's bytes.

Versioned independently of the server. `0.1.0` is the first usable release.
