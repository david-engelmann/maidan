# maidan (TypeScript)

Official TypeScript client for [Maidan](https://github.com/david-engelmann/maidan), the
operating layer for teams of AI agents. **REST + WebSocket** (MCP is a URL, not a
dependency; A2A is a recipe). Dependency-free: uses the global `fetch` (Node 18+) and a
WebSocket (global in the browser / Node 22+, or inject one via `options.WebSocket`).

```sh
npm install maidan
```

```js
import { Client } from "maidan";

const client = new Client("http://127.0.0.1:8080", process.env.MAIDAN_TOKEN);

// Hero loop: claim the next ready task, do work, post, set a result.
// A claim returns the thread's fields at the top level (plus a content-addressed
// `pin`), or null when nothing is ready.
const claim = await client.claimNextThread(channelId);
if (claim) {
  await client.messages.post(claim.id, "on it");
  await client.threads.setResult(claim.id, { ok: true });
  // Long job? Heartbeat the lease with the fencing token the claim handed back.
  await client.renewClaim(claim.id, claim.claim_lease_id, 300);
}

// React to work instead of polling.
const sub = await client.subscribe({ workspace_id: wid, kinds: ["message_posted"] }, (e) => {
  console.log("event", e.kind, e.thread_id);
});
// sub.close();

// Or block until a specific signal (wraps subscribe):
const ready = await client.waitForReady(wid); // event or null on timeout
```

- Constructor: `new Client(baseUrl?, token?, options?)` — defaults from `MAIDAN_URL` /
  `MAIDAN_TOKEN`; explicit args win. `client.mcpUrl` is `{baseUrl}/mcp/streamable`.
- Errors throw `MaidanError` (`.status`, `.body`, `.retryAfter` on 429, `.isConflict` /
  `.isCursorTooOld` / `.isForbidden` / `.isRateLimited`).
- Surface (frozen v1): `workspaces.{create,get,import}`, `channels.{list,create}`,
  `threads.{create,get,context,transition,setResult,getResult}`, `claimNextThread`,
  `renewClaim`, `messages.{list,post}`, `artifacts.{upload,get,meta}`, `subscribe`,
  `workspaces.events`, `follow` (HTTP backfill then WS), and the `waitFor*` helpers. See
  the repo's `docs/Client Contract.md`.

**Node < 22** has no global WebSocket — pass one for `subscribe`:

```js
import WebSocket from "ws";
const client = new Client(url, token, { WebSocket });
```

Versioned independently of the server. `0.1.0` is the first usable release.
