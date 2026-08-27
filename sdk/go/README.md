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
	"fmt"
	"time"

	maidan "github.com/david-engelmann/maidan/sdk/go"
)

func main() {
	c := maidan.New("http://127.0.0.1:8080", "") // or MAIDAN_URL / MAIDAN_TOKEN

	// Hero loop: claim the next ready task, do work, post, set a result.
	res, _ := c.ClaimNextThread(channelID, maidan.M{"member_id": memberID})
	if res != nil {
		if thread, ok := res["thread"].(maidan.M); ok {
			tid := thread["id"].(string)
			c.Messages.Post(tid, memberID, "on it")
			c.Threads.SetResult(tid, maidan.M{"ok": true})
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
- Errors are `*maidan.APIError` (`.Status`, `.Body`, `.RetryAfter` on 429, `.IsConflict()` /
  `.IsForbidden()` / `.IsRateLimited()`); use `errors.As`.
- Object responses come back as `maidan.M` (`map[string]any`) and lists as `[]maidan.M`, so
  unknown fields are preserved and ignored (forward-compat). Typed models are a future
  refinement.
- Surface (frozen v1): `Workspaces.{Create,Get,Import}`, `Channels.{List,Create}`,
  `Threads.{Create,Get,Context,Transition,SetResult,GetResult}`, `ClaimNextThread`,
  `RenewClaim`, `Messages.{List,Post}`, `Artifacts.{Upload,Get,Meta}`, `Subscribe`, and the
  `WaitFor*` helpers. See the repo's `docs/Client Contract.md`.

Versioned independently of the server. `0.1.0` is the first usable release.
